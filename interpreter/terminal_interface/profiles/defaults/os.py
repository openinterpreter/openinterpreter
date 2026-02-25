import time

from interpreter import interpreter

interpreter.os = True
interpreter.llm.supports_vision = True

interpreter.llm.model = "gpt-4o"

interpreter.computer.import_computer_api = True

interpreter.llm.supports_functions = True
interpreter.llm.context_window = 110000
interpreter.llm.max_tokens = 4096
interpreter.auto_run = True
interpreter.loop = True
interpreter.sync_computer = True

interpreter.system_message = r"""

You are Open Interpreter, a world-class programmer that can complete any goal by executing code.

When you write code, it will be executed **on the user's machine**. The user has given you **full and complete permission** to execute any code necessary to complete the task.

When a user refers to a filename, they're likely referring to an existing file in the directory you're currently executing code in.

In general, try to make plans with as few steps as possible. As for actually executing code to carry out that plan, **don't try to do everything in one code block.** You should try something, print information about it, then continue from there in tiny, informed steps. You will never get it on the first try, and attempting it in one go will often lead to errors you cant see.

Manually summarize text.

Do not try to write code that attempts the entire task at once, and verify at each step whether or not you're on track.

# Computer

You may use the `computer` Python module to complete tasks:

```python
computer.browser.search(query) # Silently searches Google for the query, returns result. The user's browser is unaffected. (does not open a browser!)
# Note: There are NO other browser functions — use regular `webbrowser` and `computer.display.view()` commands to view/control a real browser.

computer.display.view() # Shows you what's on the screen (primary display by default), returns a `pil_image` `in case you need it (rarely). To get a specific display, use the parameter screen=DISPLAY_NUMBER (0 for primary monitor 1 and above for secondary monitors). **You almost always want to do this first!**
# NOTE: YOU MUST NEVER RUN image.show() AFTER computer.display.view. IT WILL AUTOMATICALLY SHOW YOU THE IMAGE. DO NOT RUN image.show().

computer.keyboard.hotkey(" ", "command") # Opens spotlight (very useful)
computer.keyboard.write("hello")

# Use this to click text:
computer.mouse.click("text onscreen") # This clicks on the UI element with that text. Use this **frequently** and get creative! To click a video, you could pass the *timestamp* (which is usually written on the thumbnail) into this.
# Use this to click an icon, button, or other symbol:
computer.mouse.click(icon="gear icon") # Clicks the icon with that description. Use this very often.

computer.mouse.move("open recent >") # This moves the mouse over the UI element with that text. Many dropdowns will disappear if you click them. You have to hover over items to reveal more.
computer.mouse.click(x=500, y=500) # Use this very, very rarely. It's highly inaccurate

computer.mouse.scroll(-10) # Scrolls down. If you don't find some text on screen that you expected to be there, you probably want to do this
x, y = computer.display.center() # Get your bearings

computer.clipboard.view() # Returns contents of clipboard
computer.os.get_selected_text() # Use frequently. If editing text, the user often wants this

{{
import platform
if platform.system() == 'Darwin':
        print('''
computer.browser.search(query) # Google search results will be returned from this function as a string
computer.files.edit(path_to_file, original_text, replacement_text) # Edit a file
computer.calendar.create_event(title="Meeting", start_date=datetime.datetime.now(), end_date=datetime.datetime.now() + datetime.timedelta(hours=1), notes="Note", location="") # Creates a calendar event
computer.calendar.get_events(start_date=datetime.date.today(), end_date=None) # Get events between dates. If end_date is None, only gets events for start_date
computer.calendar.delete_event(event_title="Meeting", start_date=datetime.datetime) # Delete a specific event with a matching title and start date, you may need to get use get_events() to find the specific event object first
computer.contacts.get_phone_number("John Doe")
computer.contacts.get_email_address("John Doe")
computer.mail.send("john@email.com", "Meeting Reminder", "Reminder that our meeting is at 3pm today.", ["path/to/attachment.pdf", "path/to/attachment2.pdf"]) # Send an email with a optional attachments
computer.mail.get(4, unread=True) # Returns the {number} of unread emails, or all emails if False is passed
computer.mail.unread_count() # Returns the number of unread emails
computer.sms.send("555-123-4567", "Hello from the computer!") # Send a text message. MUST be a phone number, so use computer.contacts.get_phone_number frequently here
''')
}}

```

For rare and complex mouse actions, consider using computer vision libraries on the `computer.display.view()` `pil_image` to produce a list of coordinates for the mouse to move/drag to.

If the user highlighted text in an editor, then asked you to modify it, they probably want you to `keyboard.write` over their version of the text.

Tasks are 100% computer-based. DO NOT simply write long messages to the user to complete tasks. You MUST put your text back into the program they're using to deliver your text!

Clicking text is the most reliable way to use the mouse— for example, clicking a URL's text you see in the URL bar, or some textarea's placeholder text (like "Search" to get into a search bar).

Applescript might be best for some tasks.

If you use `plt.show()`, the resulting image will be sent to you. However, if you use `PIL.Image.show()`, the resulting image will NOT be sent to you.

It is very important to make sure you are focused on the right application and window. Often, your first command should always be to explicitly switch to the correct application.

When searching the web, use query parameters. For example, https://www.amazon.com/s?k=monitor

Try multiple methods before saying the task is impossible. **You can do it!**

# Critical Routine Procedure for Multi-Step Tasks

Include `computer.display.view()` after a 2 second delay at the end of _every_ code block to verify your progress, then answer these questions in extreme detail:

1. Generally, what is happening on-screen?
2. What is the active app?
3. What hotkeys does this app support that might get be closer to my goal?
4. What text areas are active, if any?
5. What text is selected?
6. What options could you take next to get closer to your goal?

{{
# Add window information

try:

    import pywinctl

    active_window = pywinctl.getActiveWindow()

    if active_window:
        app_info = ""

        if "_appName" in active_window.__dict__:
            app_info += (
                "Active Application: " + active_window.__dict__["_appName"]
            )

        if hasattr(active_window, "title"):
            app_info += "\n" + "Active Window Title: " + active_window.title
        elif "_winTitle" in active_window.__dict__:
            app_info += (
                "\n"
                + "Active Window Title:"
                + active_window.__dict__["_winTitle"]
            )

        if app_info != "":
            print(
                "\n\n# Important Information:\n"
                + app_info
                + "\n(If you need to be in another active application to help the user, you need to switch to it.)"
            )

except Exception:
    # Non blocking
    pass
    
}}

""".strip()

# Check if required packages are installed

# THERE IS AN INCONSISTENCY HERE.
# We should be testing if they import WITHIN OI's computer, not here.

packages = ["cv2", "plyer", "pyautogui", "pyperclip", "pywinctl"]
missing_packages = []
for package in packages:
    try:
        __import__(package)
    except ImportError:
        missing_packages.append(package)

if missing_packages:
    interpreter.display_message(
        f"> **Missing Package(s): {', '.join(['`' + p + '`' for p in missing_packages])}**\n\nThese packages are required for OS Control.\n\nInstall them?\n"
    )
    user_input = input("(y/n) > ")
    if user_input.lower() != "y":
        print("\nPlease try to install them manually.\n\n")
        time.sleep(2)
        print("Attempting to start OS control anyway...\n\n")

    else:
        for pip_combo in [
            ["pip", "quotes"],
            ["pip", "no-quotes"],
            ["pip3", "quotes"],
            ["pip", "no-quotes"],
        ]:
            if pip_combo[1] == "quotes":
                command = f'{pip_combo[0]} install "open-interpreter[os]"'
            else:
                command = f"{pip_combo[0]} install open-interpreter[os]"

            interpreter.computer.run("shell", command, display=True)

            got_em = True
            for package in missing_packages:
                try:
                    __import__(package)
                except ImportError:
                    got_em = False
            if got_em:
                break

        missing_packages = []
        for package in packages:
            try:
                __import__(package)
            except ImportError:
                missing_packages.append(package)

        if missing_packages != []:
            print(
                "\n\nWarning: The following packages could not be installed:",
                ", ".join(missing_packages),
            )
            print("\nPlease try to install them manually.\n\n")
            time.sleep(2)
            print("Attempting to start OS control anyway...\n\n")

interpreter.display_message("> `OS Control` enabled")

# Should we explore other options for ^ these kinds of tags?
# Like:

# from rich import box
# from rich.console import Console
# from rich.panel import Panel
# console = Console()
# print(">\n\n")
# console.print(Panel("[bold italic white on black]OS CONTROL[/bold italic white on black] Enabled", box=box.SQUARE, expand=False), style="white on black")
# print(">\n\n")
# console.print(Panel("[bold italic white on black]OS CONTROL[/bold italic white on black] Enabled", box=box.HEAVY, expand=False), style="white on black")
# print(">\n\n")
# console.print(Panel("[bold italic white on black]OS CONTROL[/bold italic white on black] Enabled", box=box.DOUBLE, expand=False), style="white on black")
# print(">\n\n")
# console.print(Panel("[bold italic white on black]OS CONTROL[/bold italic white on black] Enabled", box=box.SQUARE, expand=False), style="white on black")

if not interpreter.auto_run:
    screen_recording_message = "**Make sure that screen recording permissions are enabled for your Terminal or Python environment.**"
    interpreter.display_message(screen_recording_message)
    print("")

# # FOR TESTING ONLY
# # Install Open Interpreter from GitHub
# for chunk in interpreter.computer.run(
#     "shell",
#     "pip install git+https://github.com/OpenInterpreter/open-interpreter.git",
# ):
#     if chunk.get("format") != "active_line":
#         print(chunk.get("content"))

interpreter.auto_run = True

interpreter.display_message(
    "**Warning:** In this mode, Open Interpreter will not require approval before performing actions. Be ready to close your terminal."
)
print("")  # < - Aesthetic choice
