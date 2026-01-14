# Roadmap

## Documentation

- [ ] Work with Mintlify to translate docs. How does Mintlify let us translate our documentation automatically? I know there's a way.
- [ ] Better comments throughout the package (they're like docs for contributors)
- [ ] Show how to replace interpreter.llm so you can use a custom llm

## New features

- [ ] Figure out how to get OI to answer to user input requests like python's `input()`. Do we somehow detect a delay in the output..? Is there some universal flag that TUIs emit when they expect user input? Should we do this semantically with embeddings, then ask OI to review it and respond..?
- [ ] Placeholder text that gives a compelling example OI request. Probably use `textual`
- [ ] Everything else `textual` offers, like could we make it easier to select text? Copy paste in and out? Code editing interface?
- [x] Let people turn off the active line highlighting
- [x] Add a --plain flag which doesn't use rich, just prints stuff in plain text
- [ ] Use iPython stuff to track the active line, instead of inserting print statements, which makes debugging weird (From ChatGPT: For deeper insights into what's happening behind the scenes, including which line of code is being executed, you can increase the logging level of the IPython kernel. You can configure the kernel's logger to a more verbose setting, which logs each execution request. However, this requires modifying the kernel's startup settings, which might involve changing logging configurations in the IPython kernel source or when launching the kernel.)
- [ ] Let people edit the code OI writes. Could just open it in the user's preferred editor. Simple. [Full description of how to implement this here.](https://github.com/OpenInterpreter/open-interpreter/pull/830#issuecomment-1854989795)
- [ ] Display images in the terminal interface
- [x] There should be a function that just renders messages to the terminal, so we can revive conversation navigator, and let people look at their conversations
- [x] ^ This function should also render the last like 5 messages once input() is about to be run, so we don't get those weird stuttering `rich` artifacts
- [x] Let OI use OI, add `interpreter.chat(async=True)` bool. OI can use this to open OI on a new thread (Implemented as `blocking=False`)
  - [x] Also add `interpreter.await()` (Implemented as `interpreter.wait()`)
- [ ] Allow for limited functions (`interpreter.functions`) using regex
  - [ ] If `interpreter.functions != []`:
    - [ ] set `interpreter.computer.languages` to only use Python
    - [ ] Use regex to ensure the output of code blocks conforms to just using those functions + other python basics
- [ ] (Maybe) Allow for a custom embedding function (`interpreter.embed` or `computer.ai.embed`) which will let us do semantic search
- [ ] (Maybe) if a git is detected, switch to a mode that's good for developers, like showing nested file structure in dynamic system message, searching for relevant functions (use computer.files.search)
- [x] Allow for integrations somehow (you can replace interpreter.llm.completions with a wrapped completions endpoint for any kind of logging. need to document this tho)
  - [x] Document this^
- [ ] Expand "safe mode" to have proper, simple Docker support, or maybe Cosmopolitan LibC (Partially implemented in docs/integrations/docker.mdx)
- [ ] Make it so core can be run elsewhere from terminal package — perhaps split over HTTP (this would make docker easier too)
- [ ] For OS mode, experiment with screenshot just returning active window, experiment with it just showing the changes, or showing changes in addition to the whole thing, etc. GAIA should be your guide
- [x] Support for Mistral's "Ministral" models (Added `--ministral` flag and profile)
- [x] Integration of "Ultron" models via Ollama (Added `--ultron` flag and profile)
- [x] Secured local-first defaults (Removed OpenRouter dependency from default profiles)

## 2026 Milestones

- [ ] **Advanced OS Control**: Deep integration with Linux DBus (GNOME/KDE) for window management without OCR lag.
- [ ] **Persistent Memory**: Implementation of a local vector database for long-term memory and skill retrieval.
- [ ] **Multi-Agent Swarms**: Ability for one interpreter instance to spawn and manage sub-instances for parallel task execution.
- [ ] **Native Desktop App**: A cross-platform GUI that retains the speed of the terminal but adds rich visualization (Charts, Images, Interactive UI).

## Future-proofing

- [ ] Really good tests / optimization framework, to be run less frequently than Github actions tests
  - [x] Figure out how to run us on [GAIA](https://huggingface.co/gaia-benchmark)
    - [x] How do we just get the questions out of this thing?
    - [x] How do we assess whether or not OI has solved the task?
  - [ ] Loop over GAIA, use a different language model every time (use Replicate, then ask LiteLLM how they made their "mega key" to many different LLM providers)
  - [ ] Loop over that ↑ using a different prompt each time. Which prompt is best across all LLMs?
  - [ ] (For the NCU) might be good to use a Google VM with a display
  - [ ] (Future future) Use GPT-4 to assess each result, explaining each failure. Summarize. Send it all to GPT-4 + our prompt. Let it redesign the prompt, given the failures, rinse and repeat
- [ ] Stateless (as in, doesn't use the application directory) core python package. All `appdir` or `platformdirs` stuff should be only for the TUI
  - [ ] `interpreter.__dict__` = a dict derived from config is how the python package should be set, and this should be from the TUI. `interpreter` should not know about the config
  - [ ] Move conversation storage out of the core and into the TUI. When we exit or error, save messages same as core currently does
- [ ] Further split TUI from core (some utils still reach across)
- [ ] Better storage of different model keys in TUI / config file. All keys, to multiple providers, should be stored in there. Easy switching
  - [ ] Automatically migrate users from old config to new config, display a message of this
- [ ] On update, check for new system message and ask user to overwrite theirs, or only let users pass in "custom instructions" which adds to our system message
  - [ ] I think we could have a config that's like... system_message_version. If system_message_version is below the current version, ask the user if we can overwrite it with the default config system message of that version. (This somewhat exists now but needs to be robust)

# What's in our scope?

Open Interpreter contains two projects which support each other, whose scopes are as follows:

1. `core`, which is dedicated to figuring out how to get LLMs to safely control a computer. Right now, this means creating a real-time code execution environment that language models can operate.
2. `terminal_interface`, a text-only way for users to direct the code-running LLM running inside `core`. This includes functions for connecting the `core` to various local and hosted LLMs (which the `core` itself should not know about).

# What's not in our scope?

Our guiding philosophy is minimalism, so we have also decided to explicitly consider the following as **out of scope**:

1. Additional functions in `core` beyond running code.
2. More complex interactions with the LLM in `terminal_interface` beyond text (but file paths to more complex inputs, like images or video, can be included in that text).

---

This roadmap gets pretty rough from here. More like working notes.

# Working Notes

## \* Roughly, how to build `computer.browser`

First I think we should have a part, like `computer.browser.ask(query)` which just hits up [perplexity](https://www.perplexity.ai/) for fast answers to questions.

Then we want these sorts of things:

- `browser.open(url)`
- `browser.screenshot()`
- `browser.click()`

It should actually be based closely on Selenium. Copy their API so the LLM knows it.

Other than that, basically should be = to the computer module itself, at least the IO / keyboard and mouse parts.

However, for non vision models, `browser.screenshot()` can return the accessibility tree, not an image. And for `browser.click(some text)` we can use the HTML to find that text.

**Here's how GPT suggests we implement the first steps of this:**

Creating a Python script that automates the opening of Chrome with the necessary flags and then interacts with it to navigate to a URL and retrieve the accessibility tree involves a few steps. Here's a comprehensive approach:

1. **Script to Launch Chrome with Remote Debugging**:

   - This script will start Chrome with the `--remote-debugging-port=9222` flag.
   - It will handle different platforms (Windows, macOS, Linux).

2. **Python Script for Automation**:
   - This script uses `pychrome` to connect to the Chrome instance, navigate to a URL, and retrieve the accessibility tree.

### Step 1: Launching Chrome with Remote Debugging

You'll need a script to launch Chrome. This script varies based on the operating system. Below is an example for Windows. You can adapt it for macOS or Linux by changing the path and command to start Chrome.

```python
import subprocess
import sys
import os

def launch_chrome():
    chrome_path = "C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe"  # Update this path for your system
    url = "http://localhost:9222/json/version"
    subprocess.Popen([chrome_path, '--remote-debugging-port=9222'], shell=True)
    print("Chrome launched with remote debugging on port 9222.")

if __name__ == "__main__":
    launch_chrome()
```

### Step 2: Python Script to Navigate and Retrieve Accessibility Tree

Next, you'll use `pychrome` to connect to this Chrome instance. Ensure you've installed `pychrome`:

```bash
pip install pychrome
```

Here's the Python script:

```python
import pychrome
import time

def get_accessibility_tree(tab):
    # Enable the Accessibility domain
    tab.call_method("Accessibility.enable")

    # Get the accessibility tree
    tree = tab.call_method("Accessibility.getFullAXTree")
    return tree

def main():
    # Create a browser instance
    browser = pychrome.Browser(url="http://127.0.0.1:9222")

    # Create a new tab
    tab = browser.new_tab()

    # Start the tab
    tab.start()

    # Navigate to a URL
    tab.set_url("https://www.example.com")
    time.sleep(3)  # Wait for page to load

    # Retrieve the accessibility tree
    accessibility_tree = get_accessibility_tree(tab)
    print(accessibility_tree)

    # Stop the tab (closes it)
    tab.stop()

    # Close the browser
    browser.close()

if __name__ == "__main__":
    main()
```

This script will launch Chrome, connect to it, navigate to "<https://www.example.com>", and then print the accessibility tree to the console.

**Note**: The script to launch Chrome assumes a typical installation path on Windows. You will need to modify this path according to your Chrome installation location and operating system. Additionally, handling different operating systems requires conditional checks and respective commands for each OS.
