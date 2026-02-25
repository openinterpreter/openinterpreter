import os
import platform
import time

from ...utils.lazy_import import lazy_import

# Lazy import of pyautogui
pyautogui = lazy_import("pyautogui")


class Keyboard:
    """A class to simulate keyboard inputs"""

    def __init__(self, computer):
        self.computer = computer

    def write(self, text, interval=None, delay=0.30, **kwargs):
        """
        Type out a string of characters with some realistic delay.
        """
        time.sleep(delay / 2)

        if interval:
            pyautogui.write(text, interval=interval)
        else:
            try:
                clipboard_history = self.computer.clipboard.view()
            except Exception:
                pass

            ends_in_enter = False

            if text.endswith("\n"):
                ends_in_enter = True
                text = text[:-1]

            lines = text.split("\n")

            if len(lines) < 5:
                for i, line in enumerate(lines):
                    line = line + "\n" if i != len(lines) - 1 else line
                    self.computer.clipboard.copy(line)
                    self.computer.clipboard.paste()
            else:
                # just do it all at once
                self.computer.clipboard.copy(text)
                self.computer.clipboard.paste()

            if ends_in_enter:
                self.press("enter")

            try:
                self.computer.clipboard.copy(clipboard_history)
            except Exception:
                pass

        time.sleep(delay / 2)

    def press(self, *args, presses=1, interval=0.1):
        keys = args
        """
        Press a key or a sequence of keys.

        If keys is a string, it is treated as a single key and is pressed the number of times specified by presses.
        If keys is a list, each key in the list is pressed once.
        """
        time.sleep(0.15)
        pyautogui.press(keys, presses=presses, interval=interval)
        time.sleep(0.15)

    def press_and_release(self, *args, presses=1, interval=0.1):
        """
        Press and release a key or a sequence of keys.

        This method is a perfect proxy for the press method.
        """
        return self.press(*args, presses=presses, interval=interval)

    def hotkey(self, *args, interval=0.1):
        """
        Press a sequence of keys in the order they are provided, and then release them in reverse order.
        """
        time.sleep(0.15)
        modifiers = ["command", "option", "alt", "ctrl", "shift"]
        if "darwin" in platform.system().lower() and len(args) == 2:
            # pyautogui.hotkey seems to not work, so we use applescript
            # Determine which argument is the keystroke and which is the modifier
            keystroke, modifier = (
                args if args[0].lower() not in modifiers else args[::-1]
            )

            modifier = modifier.lower()

            # Map the modifier to the one that AppleScript expects
            if " down" not in modifier:
                modifier = modifier + " down"

            if keystroke.lower() == "space":
                keystroke = " "

            if keystroke.lower() == "enter":
                keystroke = "\n"

            # Create the AppleScript
            script = f"""
            tell application "System Events"
                keystroke "{keystroke}" using {modifier}
            end tell
            """

            # Execute the AppleScript
            os.system("osascript -e '{}'".format(script))
        else:
            pyautogui.hotkey(*args, interval=interval)
        time.sleep(0.15)

    def down(self, key):
        """
        Press down a key.
        """
        time.sleep(0.15)
        pyautogui.keyDown(key)
        time.sleep(0.15)

    def up(self, key):
        """
        Release a key.
        """
        time.sleep(0.15)
        pyautogui.keyUp(key)
        time.sleep(0.15)
