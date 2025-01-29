from concurrent.futures import ThreadPoolExecutor

import tiktoken


def split_into_chunks(text, tokens, llm, overlap):
    try:
        encoding = tiktoken.encoding_for_model(llm.model)
        tokenized_text = encoding.encode(text)
        chunks = []
        for i in range(0, len(tokenized_text), tokens - overlap):
            chunk = encoding.decode(tokenized_text[i : i + tokens])
            chunks.append(chunk)
    except Exception:
        chunks = []
        for i in range(0, len(text), tokens * 4 - overlap):
            chunk = text[i : i + tokens * 4]
            chunks.append(chunk)
    return chunks


def chunk_responses(responses, tokens, llm):
    try:
        encoding = tiktoken.encoding_for_model(llm.model)
        chunked_responses = []
        current_chunk = ""
        current_tokens = 0

        for response in responses:
            tokenized_response = encoding.encode(response)
            new_tokens = current_tokens + len(tokenized_response)

            # If the new token count exceeds the limit, handle the current chunk
            if new_tokens > tokens:
                # If current chunk is empty or response alone exceeds limit, add response as standalone
                if current_tokens == 0 or len(tokenized_response) > tokens:
                    chunked_responses.append(response)
                else:
                    chunked_responses.append(current_chunk)
                    current_chunk = response
                    current_tokens = len(tokenized_response)
                continue

            # Add response to the current chunk
            current_chunk += "\n\n" + response if current_chunk else response
            current_tokens = new_tokens

        # Add remaining chunk if not empty
        if current_chunk:
            chunked_responses.append(current_chunk)
    except Exception:
        chunked_responses = []
        current_chunk = ""
        current_chars = 0

        for response in responses:
            new_chars = current_chars + len(response)

            # If the new char count exceeds the limit, handle the current chunk
            if new_chars > tokens * 4:
                # If current chunk is empty or response alone exceeds limit, add response as standalone
                if current_chars == 0 or len(response) > tokens * 4:
                    chunked_responses.append(response)
                else:
                    chunked_responses.append(current_chunk)
                    current_chunk = response
                    current_chars = len(response)
                continue

            # Add response to the current chunk
            current_chunk += "\n\n" + response if current_chunk else response
            current_chars = new_chars

        # Add remaining chunk if not empty
        if current_chunk:
            chunked_responses.append(current_chunk)
    return chunked_responses


def fast_llm(llm, system_message, user_message):
    """
    Creates a temporary chat context to process a single query, then restores the original chat state.

    This is used for auxiliary queries (like summarization) that shouldn't affect the main conversation.
    Particularly important for local LLMs where creating new instances is expensive.

    Args:
        llm: The LLM instance to use (typically computer.interpreter.llm)
        system_message: The system prompt for this specific query
        user_message: The user message/content to process

    Returns:
        str: The LLM's response content

    Note:
        This function temporarily replaces the LLM's conversation state (messages and system prompt),
        runs the query, then restores the original state. This allows us to run one-off queries
        without disrupting the main conversation context.
    """
    old_messages = llm.interpreter.messages
    old_system_message = llm.interpreter.system_message
    try:
        llm.interpreter.system_message = system_message
        llm.interpreter.messages = []
        response = llm.interpreter.chat(user_message)
        return response[-1].get("content")
    finally:
        # Always restore the old state (before returning)
        llm.interpreter.messages = old_messages
        llm.interpreter.system_message = old_system_message


def query_map_chunks(chunks, llm, query):
    """Query the chunks of text using query_chunk_map."""
    with ThreadPoolExecutor() as executor:
        responses = list(
            executor.map(lambda chunk: fast_llm(llm, query, chunk), chunks)
        )
    return responses


def query_reduce_chunks(responses, llm, chunk_size, query):
    """Reduce query responses in a while loop."""
    while len(responses) > 1:
        chunks = chunk_responses(responses, chunk_size, llm)

        # Use multithreading to summarize each chunk simultaneously
        with ThreadPoolExecutor() as executor:
            summaries = list(
                executor.map(lambda chunk: fast_llm(llm, query, chunk), chunks)
            )

    return summaries[0]


class Ai:
    def __init__(self, computer):
        self.computer = computer

    def chat(self, text, base64=None):
        messages = [
            {
                "role": "system",
                "type": "message",
                "content": "You are a helpful AI assistant.",
            },
            {"role": "user", "type": "message", "content": text},
        ]
        if base64:
            messages.append(
                {"role": "user", "type": "image", "format": "base64", "content": base64}
            )
        response = ""
        for chunk in self.computer.interpreter.llm.run(messages):
            if "content" in chunk:
                response += chunk.get("content")
        return response

        # Old way
        old_messages = self.computer.interpreter.llm.interpreter.messages
        old_system_message = self.computer.interpreter.llm.interpreter.system_message
        old_import_computer_api = self.computer.import_computer_api
        old_execution_instructions = (
            self.computer.interpreter.llm.execution_instructions
        )
        try:
            self.computer.interpreter.llm.interpreter.system_message = (
                "You are an AI assistant."
            )
            self.computer.interpreter.llm.interpreter.messages = []
            self.computer.import_computer_api = False
            self.computer.interpreter.llm.execution_instructions = ""

            response = self.computer.interpreter.llm.interpreter.chat(text)
        finally:
            self.computer.interpreter.llm.interpreter.messages = old_messages
            self.computer.interpreter.llm.interpreter.system_message = (
                old_system_message
            )
            self.computer.import_computer_api = old_import_computer_api
            self.computer.interpreter.llm.execution_instructions = (
                old_execution_instructions
            )

            return response[-1].get("content")

    def query(self, text, query, custom_reduce_query=None):
        if custom_reduce_query == None:
            custom_reduce_query = query

        chunk_size = 2000
        overlap = 50

        # Split the text into chunks
        chunks = split_into_chunks(
            text, chunk_size, self.computer.interpreter.llm, overlap
        )

        # (Map) Query each chunk
        responses = query_map_chunks(chunks, self.computer.interpreter.llm, query)

        # (Reduce) Compress the responses
        response = query_reduce_chunks(
            responses, self.computer.interpreter.llm, chunk_size, custom_reduce_query
        )

        return response

    def summarize(self, text):
        query = "You are a highly skilled AI trained in language comprehension and summarization. I would like you to read the following text and summarize it into a concise abstract paragraph. Aim to retain the most important points, providing a coherent and readable summary that could help a person understand the main points of the discussion without needing to read the entire text. Please avoid unnecessary details or tangential points."
        custom_reduce_query = "You are tasked with taking multiple summarized texts and merging them into one unified and concise summary. Maintain the core essence of the content and provide a clear and comprehensive summary that encapsulates all the main points from the individual summaries."
        return self.query(text, query, custom_reduce_query)
