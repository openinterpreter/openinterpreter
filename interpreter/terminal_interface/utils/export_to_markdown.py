def export_to_markdown(messages: list[dict], export_path: str):
    markdown = messages_to_markdown(messages)
    # Force UTF-8 so emoji / CJK / accented characters export correctly. Without
    # this, text mode falls back to the locale codepage (cp1252 on Windows),
    # which raises UnicodeEncodeError on non-Latin-1 content.
    with open(export_path, "w", encoding="utf-8") as f:
        f.write(markdown)
    print(f"Exported current conversation to {export_path}")


def messages_to_markdown(messages: list[dict]) -> str:
    # Convert interpreter.messages to Markdown text
    markdown_content = ""
    previous_role = None
    for chunk in messages:
        current_role = chunk["role"]
        if current_role == previous_role:
            rendered_chunk = ""
        else:
            rendered_chunk = f"## {current_role}\n\n"
            previous_role = current_role

        # User query message
        if chunk["role"] == "user":
            rendered_chunk += chunk["content"] + "\n\n"
            markdown_content += rendered_chunk
            continue

        # Message
        if chunk["type"] == "message":
            rendered_chunk += chunk["content"] + "\n\n"

        # Code
        if chunk["type"] == "code" or chunk["type"] == "console":
            code_format = chunk.get("format", "")
            rendered_chunk += f"```{code_format}\n{chunk['content']}\n```\n\n"

        markdown_content += rendered_chunk

    return markdown_content
