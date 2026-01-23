import re

from .subprocess_language import SubprocessLanguage


class R(SubprocessLanguage):
    file_extension = "r"
    name = "R"

    def __init__(self):
        super().__init__()
        self.start_cmd = ["R", "-q", "--vanilla"]  # Start R in quiet and vanilla mode

    def preprocess_code(self, code):
        """
        Add active line markers
        Wrap in a tryCatch for better error handling in R
        Add end of execution marker
        """

        lines = code.split("\n")
        processed_lines = []

        for i, line in enumerate(lines, 1):
            stripped = line.strip()
            lstripped = line.lstrip()

            # Skip injecting markers on non-executable structural lines.
            # Otherwise the trailing cat() becomes the last expression in a block
            # and implicit returns become NULL instead of the actual value.
            # This affects blank lines, comments, and closing tokens like }, ), ]
            if not stripped or lstripped.startswith("#"):
                processed_lines.append(line)
                continue
            if lstripped and lstripped[0] in ")]}":
                processed_lines.append(line)
                continue

            # Add active line print for executable lines
            processed_lines.append(f'cat("##active_line{i}##\\n");{line}')

        # Join lines to form the processed code
        processed_code = "\n".join(processed_lines)

        # Wrap in a tryCatch for error handling and add end of execution marker
        processed_code = f"""
tryCatch({{
{processed_code}
}}, error=function(e){{
    cat("##execution_error##\\n", conditionMessage(e), "\\n");
}})
cat("##end_of_execution##\\n");
"""
        # Count the number of lines of processed_code
        # (R echoes all code back for some reason, but we can skip it if we track this!)
        self.code_line_count = len(processed_code.split("\n")) - 1

        return processed_code

    def line_postprocessor(self, line):
        # If the line count attribute is set and non-zero, decrement and skip the line
        if hasattr(self, "code_line_count") and self.code_line_count > 0:
            self.code_line_count -= 1
            return None

        if re.match(r"^(\s*>>>\s*|\s*\.\.\.\s*|\s*>\s*|\s*\+\s*|\s*)$", line):
            return None
        if "R version" in line:  # Startup message
            return None
        if line.strip().startswith('[1] "') and line.endswith(
            '"'
        ):  # For strings, trim quotation marks
            return line[5:-1].strip()
        if line.strip().startswith(
            "[1]"
        ):  # Normal R output prefix for non-string outputs
            return line[4:].strip()

        return line

    def detect_active_line(self, line):
        if "##active_line" in line:
            return int(line.split("##active_line")[1].split("##")[0])
        return None

    def detect_end_of_execution(self, line):
        return "##end_of_execution##" in line or "##execution_error##" in line
