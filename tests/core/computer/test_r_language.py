"""Tests for R language preprocessing in interpreter."""

import unittest

from interpreter.core.computer.terminal.languages.r import R


class TestRPreprocessing(unittest.TestCase):
    """Test R code preprocessing for proper return value handling."""

    def setUp(self):
        """Set up R language instance for testing."""
        self.r = R()

    def test_active_line_markers_on_executable_lines(self):
        """Test that active line markers are added to executable lines."""
        code = "x <- 1\nprint(x)"
        processed = self.r.preprocess_code(code)
        # Both lines should have active line markers
        self.assertIn('cat("##active_line1##\\n");x <- 1', processed)
        self.assertIn('cat("##active_line2##\\n");print(x)', processed)

    def test_no_markers_on_blank_lines(self):
        """Test that blank lines don't get active line markers."""
        code = "x <- 1\n\nprint(x)"
        processed = self.r.preprocess_code(code)
        # Line 1 and 3 should have markers, but the blank line (2) should not
        self.assertIn('cat("##active_line1##\\n");x <- 1', processed)
        self.assertIn('cat("##active_line3##\\n");print(x)', processed)
        # The blank line should appear without a marker
        lines = processed.split("\n")
        # Find the blank line - it should be preserved without marker injection
        blank_line_found = any(
            line.strip() == "" and "##active_line" not in line for line in lines
        )
        self.assertTrue(blank_line_found, "Blank line should not have marker")

    def test_no_markers_on_comments(self):
        """Test that comment lines don't get active line markers."""
        code = "# This is a comment\nx <- 1"
        processed = self.r.preprocess_code(code)
        # The comment line should not have a marker injected before it
        self.assertNotIn('cat("##active_line1##\\n");# This', processed)
        # The actual code line should have a marker
        self.assertIn('cat("##active_line2##\\n");x <- 1', processed)

    def test_no_markers_on_closing_braces(self):
        """Test that closing braces don't get active line markers.

        This is the key fix for issue #1655 - implicit function returns
        returning NULL because the last expression was cat() injected
        before the closing brace.
        """
        code = "f <- function(x) {\n  x * 2\n}"
        processed = self.r.preprocess_code(code)
        # The function definition and body should have markers
        self.assertIn('cat("##active_line1##\\n");f <- function(x) {', processed)
        self.assertIn('cat("##active_line2##\\n");  x * 2', processed)
        # The closing brace should NOT have a marker
        self.assertNotIn('cat("##active_line3##\\n");}', processed)

    def test_no_markers_on_closing_parens(self):
        """Test that closing parentheses don't get active line markers."""
        code = "result <- sum(\n  c(1, 2, 3)\n)"
        processed = self.r.preprocess_code(code)
        # Closing paren line should not have marker
        self.assertNotIn('cat("##active_line3##\\n");)', processed)

    def test_no_markers_on_closing_brackets(self):
        """Test that closing brackets don't get active line markers."""
        code = "x[\n  1\n]"
        processed = self.r.preprocess_code(code)
        # Closing bracket line should not have marker
        self.assertNotIn('cat("##active_line3##\\n");]', processed)

    def test_trycatch_wrapper_present(self):
        """Test that code is wrapped in tryCatch for error handling."""
        code = "x <- 1"
        processed = self.r.preprocess_code(code)
        self.assertIn("tryCatch({", processed)
        self.assertIn("error=function(e)", processed)

    def test_end_of_execution_marker(self):
        """Test that end of execution marker is added."""
        code = "x <- 1"
        processed = self.r.preprocess_code(code)
        self.assertIn('cat("##end_of_execution##\\n");', processed)

    def test_function_with_implicit_return(self):
        """Test preprocessing of function with implicit return (issue #1655).

        The function's implicit return should not be affected by marker
        injection on the closing brace.
        """
        code = """normalize <- function(x) {
    (x - min(x)) / (max(x) - min(x))
}"""
        processed = self.r.preprocess_code(code)
        # The closing brace should not have a marker
        self.assertNotIn('cat("##active_line3##\\n");}', processed)
        # The actual computation line should have a marker
        self.assertIn("##active_line2##", processed)


if __name__ == "__main__":
    unittest.main()
