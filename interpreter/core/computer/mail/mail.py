import os
import platform
import re
import subprocess

from ..utils.run_applescript import run_applescript, run_applescript_capture


class Mail:
    def __init__(self, computer):
        self.computer = computer
        # In the future, we should allow someone to specify their own mail app
        self.mail_app = "Mail"

    def get(self, number=5, unread: bool = False):
        """
        Retrieves the last {number} emails from the inbox, optionally filtering for only unread emails.
        """
        if platform.system() != "Darwin":
            return "This method is only supported on MacOS"

        too_many_emails_msg = ""
        if number > 50:
            number = min(number, 50)
            too_many_emails_msg = (
                "This method is limited to 10 emails, returning the first 10: "
            )
        # This is set up to retry if the number of emails is less than the number requested, but only a max of three times
        retries = 0  # Initialize the retry counter
        while retries < 3:
            read_status_filter = "whose read status is false" if unread else ""
            script = f"""
            tell application "{self.mail_app}"
                set latest_messages to messages of inbox {read_status_filter}
                set email_data to {{}}
                repeat with i from 1 to {number}
                    set this_message to item i of latest_messages
                    set end of email_data to {{subject:subject of this_message, sender:sender of this_message, content:content of this_message}}
                end repeat
                return email_data
            end tell
            """
            stdout, stderr = run_applescript_capture(script)

            # if the error is due to not having enough emails, retry with the available emails.
            if "Can’t get item" in stderr:
                match = re.search(r"Can’t get item (\d+) of", stderr)
                if match:
                    available_emails = int(match.group(1)) - 1
                    if available_emails > 0:
                        number = available_emails
                        retries += 1
                        continue
                break
            elif stdout:
                if too_many_emails_msg:
                    return f"{too_many_emails_msg}\n\n{stdout}"
                else:
                    return stdout

    def send(self, to, subject, body, attachments=None):
        """
        Sends an email with the given parameters using the default mail app.
        """
        if platform.system() != "Darwin":
            return "This method is only supported on MacOS"

        # Strip newlines from the to field
        to = to.replace("\n", "")

        attachment_clause = ""
        delay_seconds = 5  # Default delay in seconds

        if attachments:
            formatted_attachments = [
                self.format_path_for_applescript(path) for path in attachments
            ]

            # Generate AppleScript to attach each file
            attachment_clause = "\n".join(
                f"make new attachment with properties {{file name:{path}}} at after the last paragraph of the content of new_message"
                for path in formatted_attachments
            )

            # Calculate the delay based on the size of the attachments
            delay_seconds = self.calculate_upload_delay(attachments)

            print(f"Uploading attachments. This should take ~{delay_seconds} seconds.")

        # In the future, we might consider allowing the llm to specify an email to send from
        script = f"""
        tell application "{self.mail_app}"
            set new_message to make new outgoing message with properties {{subject:"{subject}", content:"{body}"}} at end of outgoing messages
            tell new_message
                set visible to true
                make new to recipient at end of to recipients with properties {{address:"{to}"}}
                {attachment_clause}
            end tell
            {f'delay {delay_seconds}' if attachments else ''}
            send new_message
        end tell
        """
        try:
            run_applescript(script)
            return f"""Email sent to {to}"""
        except subprocess.CalledProcessError:
            return "Failed to send email"

    def unread_count(self):
        """
        Retrieves the count of unread emails in the inbox, limited to 50.
        """
        if platform.system() != "Darwin":
            return "This method is only supported on MacOS"

        script = f"""
            tell application "{self.mail_app}"
                set unreadMessages to (messages of inbox whose read status is false)
                if (count of unreadMessages) > 50 then
                    return 50
                else
                    return count of unreadMessages
                end if
            end tell
            """
        try:
            unreads = int(run_applescript(script))
            if unreads >= 50:
                return "50 or more"
            return unreads
        except subprocess.CalledProcessError as e:
            print(e)
            return 0

    # Estimate how long something will take to upload
    def calculate_upload_delay(self, attachments):
        try:
            total_size_mb = sum(
                os.path.getsize(os.path.expanduser(att)) for att in attachments
            ) / (1024 * 1024)
            # Assume 1 MBps upload speed, which is conservative on purpose
            upload_speed_mbps = 1
            estimated_time_seconds = total_size_mb / upload_speed_mbps
            return round(
                max(0.2, estimated_time_seconds + 1), 1
            )  # Add 1 second buffer, ensure a minimum delay of 1.2 seconds, rounded to one decimal place
        except Exception:
            # Return a default delay of 5 seconds if an error occurs
            return 5

    def format_path_for_applescript(self, file_path):
        # Escape backslashes, quotes, and curly braces for AppleScript
        file_path = (
            file_path.replace("\\", "\\\\")
            .replace('"', '\\"')
            .replace("{", "\\{")
            .replace("}", "\\}")
        )
        # Convert to a POSIX path and quote for AppleScript
        posix_path = f'POSIX file "{file_path}"'
        return posix_path
