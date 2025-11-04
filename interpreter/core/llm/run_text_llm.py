def run_text_llm(llm, params):
    ## Setup

    if llm.execution_instructions:
        try:
            # Add the system message
            params["messages"][0][
                "content"
            ] += "\n" + llm.execution_instructions
        except Exception:
            # Debug print before re-raising
            print('params["messages"][0]', params["messages"][0])
            raise

    ## Convert output to LMC format

    inside_code_block = False
    accumulated_block = ""
    language = None

    for chunk in llm.completions(**params):
        if llm.interpreter.verbose:
            print("Chunk in coding_llm", chunk)

        if "choices" not in chunk or len(chunk["choices"]) == 0:
            # This happens sometimes
            continue

        content = chunk["choices"][0]["delta"].get("content", "")

        if content == None:
            continue

        accumulated_block += content

        if accumulated_block.endswith("`"):
            # We might be writing "```" one token at a time.
            continue

        # Did we just enter a code block?
        if "```" in accumulated_block and not inside_code_block:
            inside_code_block = True
            accumulated_block = accumulated_block.split("```")[1]

        # Did we just exit a code block?
        if inside_code_block and "```" in accumulated_block:
            return

        # If we're in a code block,
        if inside_code_block:
            # If we don't have a `language`, find it
            if language is None and "\n" in accumulated_block:
                language = accumulated_block.split("\n")[0]

                # Default to python if not specified
                if language == "":
                    if llm.interpreter.os == False:
                        language = "python"
                    elif llm.interpreter.os == False:
                        # OS mode does this frequently. Takes notes with markdown code blocks
                        language = "text"
                else:
                    # Removes hallucinations containing spaces or non letters.
                    language = "".join(char for char in language if char.isalpha())

            # If we do have a `language`, send it out
            if language:
                yield {
                    "type": "code",
                    "format": language,
                    "content": content.replace(language, ""),
                }

        # If we're not in a code block, send the output as a message
        if not inside_code_block:
            yield {"type": "message", "content": content}
