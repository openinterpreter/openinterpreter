import datetime
import os
import plistlib
import sqlite3
import subprocess
import sys
import time


class SMS:
    def __init__(self, computer):
        self.computer = computer
        if sys.platform.lower() == "darwin":  # Only if macOS
            self.database_path = self.resolve_database_path()
        else:
            self.database_path = None

    def resolve_database_path(self):
        try:
            if os.geteuid() == 0:  # Running as root
                home_directory = os.path.expanduser(f"~{os.environ.get('SUDO_USER')}")
            else:
                home_directory = os.path.expanduser("~")
            return f"{home_directory}/Library/Messages/chat.db"
        except Exception:
            home_directory = os.path.expanduser("~")
            return f"{home_directory}/Library/Messages/chat.db"

    def send(self, to, message):
        if sys.platform.lower() != "darwin":
            print("Only supported on Mac.")
            return
        message_escaped = message.replace('"', '\\"').replace("\\", "\\\\")
        script = f"""
        tell application "Messages"
            set targetBuddy to "{to}"
            send "{message_escaped}" to buddy targetBuddy of (service 1 whose service type is iMessage)
        end tell
        """
        subprocess.run(["osascript", "-e", script], check=True)
        return "Message sent successfully"

    def get(self, contact=None, limit=10, substring=None):
        if sys.platform.lower() != "darwin":
            print("Only supported on Mac.")
            return
        if not self.can_access_database():
            self.prompt_full_disk_access()

        conn = sqlite3.connect(self.database_path)
        conn.row_factory = sqlite3.Row  # Set row factory
        cursor = conn.cursor()
        query = """
SELECT message.*, handle.id as sender FROM message
LEFT JOIN handle ON message.handle_id = handle.ROWID
        """
        params = []
        conditions = []

        if contact:
            conditions.append("handle.id=?")
            params.append(contact)
        if substring:
            conditions.append("message.text LIKE ?")
            params.append(f"%{substring}%")
        if conditions:
            query += " WHERE " + " AND ".join(conditions)
        query += " ORDER BY message.date DESC"

        cursor.execute(query, params)

        # Parse plist data and make messages readable
        readable_messages = []
        while len(readable_messages) < limit:
            try:
                message = cursor.fetchone()
                if message is None:
                    break
                message_dict = dict(message)  # Convert row to dictionary
                text_data = message_dict.get("text")
                if text_data:
                    try:
                        # Try to parse as plist
                        plist_data = plistlib.loads(text_data)
                        text = plist_data.get("NS.string", "")
                    except Exception:
                        # If plist parsing fails, use the raw string
                        text = text_data
                    if text:  # Only add messages with content
                        # Convert Apple timestamp to datetime
                        date = datetime.datetime(2001, 1, 1) + datetime.timedelta(
                            seconds=message_dict.get("date") / 10**9
                        )
                        sender = message_dict.get("sender")
                        if message_dict.get("is_from_me") == 1:
                            sender = "(Me)"
                        readable_messages.append(
                            {"date": date, "from": sender, "text": text}
                        )
            except sqlite3.Error as e:
                break

        conn.close()
        return readable_messages

    def can_access_database(self):
        try:
            with open(self.database_path, "r"):
                return True
        except IOError:
            return False

    def prompt_full_disk_access(self):
        script = """
        tell application "System Preferences"
            activate
        end tell
        delay 1
        tell application "System Events"
            display dialog "This application requires Full Disk Access to function properly.\\n\\nPlease follow these steps:\\n1. Open the Security & Privacy panel.\\n2. Go to the Full Disk Access section.\\n3. Click the lock icon and enter your password to make changes.\\n4. Click the '+' button and add your terminal application (e.g., Terminal, iTerm).\\n5. Restart the application after granting access." buttons {"OK"} default button "OK"
        end tell
        """
        subprocess.run(["osascript", "-e", script], check=True)
