try:
    import tiktoken
    from litellm import cost_per_token
except Exception:
    # Non-essential feature
    pass


def count_tokens(text="", model="gpt-4"):
    """
    Count the number of tokens in a string
    """
    try:
        # Fix bug where models starting with openai/ for example can't find tokenizer
        if "/" in model:
            model = model.split("/")[-1]

        # At least give an estimate if we can't find the tokenizer
        try:
            encoder = tiktoken.encoding_for_model(model)
        except KeyError:
            print(
                f"Could not find tokenizer for {model}. Defaulting to gpt-4 tokenizer."
            )
            encoder = tiktoken.encoding_for_model("gpt-4")

        return len(encoder.encode(text))
    except Exception:
        # Non-essential feature
        return 0


def token_cost(tokens=0, model="gpt-4"):
    """
    Calculate the cost of the current number of tokens
    """

    try:
        (prompt_cost, _) = cost_per_token(model=model, prompt_tokens=tokens)

        return round(prompt_cost, 6)
    except Exception:
        # Non-essential feature
        return 0


def count_messages_tokens(messages=[], model=None):
    """
    Count the number of tokens in a list of messages
    """
    try:
        tokens_used = 0

        for message in messages:
            if isinstance(message, str):
                tokens_used += count_tokens(message, model=model)
            elif "message" in message:
                tokens_used += count_tokens(message["message"], model=model)

                if "code" in message:
                    tokens_used += count_tokens(message["code"], model=model)

                if "output" in message:
                    tokens_used += count_tokens(message["output"], model=model)

        prompt_cost = token_cost(tokens_used, model=model)

        return (tokens_used, prompt_cost)
    except Exception:
        # Non-essential feature
        return (0, 0)
