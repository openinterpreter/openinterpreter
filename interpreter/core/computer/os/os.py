import platform
import subprocess


class Os:
    def __init__(self, computer):
        self.computer = computer

    def get_selected_text(self):
        """
        Returns the currently selected text.
        """
        # Store the current clipboard content
        current_clipboard = self.computer.clipboard.view()
        # Copy the selected text to clipboard
        self.computer.clipboard.copy()
        # Get the selected text from clipboard
        selected_text = self.computer.clipboard.view()
        # Reset the clipboard to its original content
        self.computer.clipboard.copy(current_clipboard)
        return selected_text

    def notify(self, text):
        """
        Displays a notification on the computer.
        """
        try:
            title = "Open Interpreter"

            if len(text) > 200:
                text = text[:200] + "..."

            if "darwin" in platform.system().lower():  # Check if the OS is macOS
                text = text.replace('"', "'").replace("\n", " ")
                text = (
                    text.replace('"', "")
                    .replace("'", "")
                    .replace("“", "")
                    .replace("”", "")
                    .replace("<", "")
                    .replace(">", "")
                    .replace("&", "")
                )

                # Further sanitize the text to avoid errors
                text = text.encode("unicode_escape").decode("utf-8")

                ## Run directly
                script = f'display notification "{text}" with title "{title}"'
                subprocess.run(["osascript", "-e", script])

                # ## DISABLED OI-notifier.app
                # (This does not work. It makes `pip uninstall`` break for some reason!)

                # ## Use OI-notifier.app, which lets us use a custom icon

                # # Get the path of the current script
                # script_path = os.path.dirname(os.path.realpath(__file__))

                # # Write the notification text into notification_text.txt
                # with open(os.path.join(script_path, "notification_text.txt"), "w") as file:
                #     file.write(text)

                # # Construct the path to the OI-notifier.app
                # notifier_path = os.path.join(script_path, "OI-notifier.app")

                # # Call the OI-notifier
                # subprocess.run(["open", notifier_path])
            else:  # For other OS, use a general notification API
                try:
                    import plyer

                    plyer.notification.notify(title=title, message=text)
                except Exception:
                    # Optional package
                    pass
        except Exception as e:
            # Notifications should be non-blocking
            if self.computer.verbose:
                print("Notification error:")
                print(str(e))

    # Maybe run code should be here...?
