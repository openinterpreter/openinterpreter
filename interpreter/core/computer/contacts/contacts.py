import platform

from ..utils.run_applescript import run_applescript_capture


class Contacts:
    def __init__(self, computer):
        self.computer = computer

    def get_phone_number(self, contact_name):
        """
        Returns the phone number of a contact by name.
        """
        if platform.system() != "Darwin":
            return "This method is only supported on MacOS"

        script = f"""
        tell application "System Events" to tell process "Finder"
            open location "addressbook://"
            tell application "Contacts"
                set thePerson to first person whose name is "{contact_name}"
                if exists thePerson then
                    set theNumber to value of first phone of thePerson
                    return theNumber
                else
                    return "Contact not found"
                end if
            end tell
        end tell
        """
        stout, stderr = run_applescript_capture(script)
        # If the person is not found, we will try to find similar contacts
        if "Can’t get person" in stderr or not stout:
            names = self.get_full_names_from_first_name(contact_name)
            if "No contacts found" in names or not names:
                raise ValueError("Contact not found")
            else:
                # Language model friendly error message
                raise ValueError(
                    f"A contact for '{contact_name}' was not found, perhaps one of these similar contacts might be what you are looking for? {names} \n Please try again and provide a more specific contact name."
                )
        else:
            return stout.replace("\n", "")

    def get_email_address(self, contact_name):
        """
        Returns the email address of a contact by name.
        """
        if platform.system() != "Darwin":
            return "This method is only supported on MacOS"

        script = f"""
        tell application "Contacts"
            set thePerson to first person whose name is "{contact_name}"
            set theEmail to value of first email of thePerson
            return theEmail
        end tell
        """
        stout, stderr = run_applescript_capture(script)
        # If the person is not found, we will try to find similar contacts
        if "Can’t get person" in stderr:
            names = self.get_full_names_from_first_name(contact_name)
            if names == "No contacts found":
                return "No contacts found"
            else:
                # Language model friendly error message
                return f"A contact for '{contact_name}' was not found, perhaps one of these similar contacts might be what you are looking for? {names} \n Please try again and provide a more specific contact name."
        else:
            return stout.replace("\n", "")

    def get_full_names_from_first_name(self, first_name):
        """
        Returns a list of full names of contacts that contain the first name provided.
        """
        if platform.system() != "Darwin":
            return "This method is only supported on MacOS"

        script = f"""
        tell application "Contacts"
            set matchingPeople to every person whose name contains "{first_name}"
            set namesList to {{}}
            repeat with aPerson in matchingPeople
                set end of namesList to name of aPerson
            end repeat
            return namesList
        end tell
        """
        names, _ = run_applescript_capture(script)
        if names:
            return names
        else:
            return "No contacts found."
