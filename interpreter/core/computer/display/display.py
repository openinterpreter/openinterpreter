import base64
import io
import os
import platform
import pprint
import subprocess
import time
import warnings
from contextlib import redirect_stdout
from io import BytesIO

import requests
from IPython.display import display
from PIL import Image

from ...utils.lazy_import import lazy_import
from ..utils.recipient_utils import format_to_recipient

# Still experimenting with this
# from utils.get_active_window import get_active_window

# Lazy import of optional packages
try:
    cv2 = lazy_import("cv2")
except Exception:
    cv2 = None  # Fixes colab error

pyautogui = lazy_import("pyautogui")

# Check if there's a display available
try:
    # Attempt to get the screen size
    pyautogui.size()
except Exception:
    pyautogui = None

np = lazy_import("numpy")
plt = lazy_import("matplotlib.pyplot")
screeninfo = lazy_import("screeninfo")
pywinctl = lazy_import("pywinctl")


from ..utils.computer_vision import find_text_in_image, pytesseract_get_text


class Display:
    def __init__(self, computer):
        self.computer = computer
        # set width and height to None initially to prevent pyautogui from importing until it's needed
        self._width = None
        self._height = None
        self._hashes = {}

    # We use properties here so that this code only executes when height/width are accessed for the first time
    @property
    def width(self):
        if self._width is None:
            self._width, _ = pyautogui.size()
        return self._width

    @property
    def height(self):
        if self._height is None:
            _, self._height = pyautogui.size()
        return self._height

    def size(self):
        """
        Returns the current screen size as a tuple (width, height).
        """
        return pyautogui.size()

    def center(self):
        """
        Calculates and returns the center point of the screen as a tuple (x, y).
        """
        return self.width // 2, self.height // 2

    def info(self):
        """
        Returns a list of all connected monitor/displays and their information
        """
        return get_displays()

    def view(
        self,
        show=True,
        quadrant=None,
        screen=0,
        combine_screens=True,
        active_app_only=True,
    ):
        """
        Redirects to self.screenshot
        """
        return self.screenshot(
            screen=screen,
            show=show,
            quadrant=quadrant,
            combine_screens=combine_screens,
            active_app_only=active_app_only,
        )

    # def get_active_window(self):
    #     return get_active_window()

    def screenshot(
        self,
        screen=0,
        show=True,
        quadrant=None,
        active_app_only=True,
        combine_screens=True,
    ):
        """
        Shows you what's on the screen by taking a screenshot of the entire screen or a specified quadrant. Returns a `pil_image` `in case you need it (rarely). **You almost always want to do this first!**
        :param screen: specify which display; 0 for primary and 1 and above for secondary.
        :param combine_screens: If True, a collage of all display screens will be returned. Otherwise, a list of display screens will be returned.
        """

        # Since Local II, all images sent to local models will be rendered to text with moondream and pytesseract.
        # So we don't need to do this here— we can just emit images.
        # We should probably remove self.computer.emit_images for this reason.

        # if not self.computer.emit_images and force_image == False:
        #     screenshot = self.screenshot(show=False, force_image=True)

        #     description = self.computer.vision.query(pil_image=screenshot)
        #     print("A DESCRIPTION OF WHAT'S ON THE SCREEN: " + description)

        #     if self.computer.max_output > 600:
        #         print("ALL OF THE TEXT ON THE SCREEN: ")
        #         text = self.get_text_as_list_of_lists(screenshot=screenshot)
        #         pp = pprint.PrettyPrinter(indent=4)
        #         pretty_text = pp.pformat(text)  # language models like it pretty!
        #         pretty_text = format_to_recipient(pretty_text, "assistant")
        #         print(pretty_text)
        #         print(
        #             format_to_recipient(
        #                 "To receive the text above as a Python object, run computer.display.get_text_as_list_of_lists()",
        #                 "assistant",
        #             )
        #         )
        #     return screenshot  # Still return a PIL image

        if quadrant == None:
            if active_app_only:
                active_window = pywinctl.getActiveWindow()
                if active_window:
                    screenshot = pyautogui.screenshot(
                        region=(
                            active_window.left,
                            active_window.top,
                            active_window.width,
                            active_window.height,
                        )
                    )
                    message = format_to_recipient(
                        "Taking a screenshot of the active app. To take a screenshot of the entire screen (uncommon), use computer.view(active_app_only=False).",
                        "assistant",
                    )
                    print(message)
                else:
                    screenshot = pyautogui.screenshot()

            else:
                screenshot = take_screenshot_to_pil(
                    screen=screen, combine_screens=combine_screens
                )  #  this function uses pyautogui.screenshot which works fine for all OS (mac, linux and windows)
                message = format_to_recipient(
                    "Taking a screenshot of the entire screen.\n\nTo focus on the active app, use computer.display.view(active_app_only=True).",
                    "assistant",
                )
                print(message)

        else:
            screen_width, screen_height = pyautogui.size()

            quadrant_width = screen_width // 2
            quadrant_height = screen_height // 2

            quadrant_coordinates = {
                1: (0, 0),
                2: (quadrant_width, 0),
                3: (0, quadrant_height),
                4: (quadrant_width, quadrant_height),
            }

            if quadrant in quadrant_coordinates:
                x, y = quadrant_coordinates[quadrant]
                screenshot = pyautogui.screenshot(
                    region=(x, y, quadrant_width, quadrant_height)
                )
            else:
                raise ValueError("Invalid quadrant. Choose between 1 and 4.")

        # Open the image file with PIL
        # IPython interactive mode auto-displays plots, causing RGBA handling issues, possibly MacOS-specific.
        if isinstance(screenshot, list):
            screenshot = [
                img.convert("RGB") for img in screenshot
            ]  # if screenshot is a list (i.e combine_screens=False).
        else:
            screenshot = screenshot.convert("RGB")

        if show:
            # Show the image using IPython display
            if isinstance(screenshot, list):
                for img in screenshot:
                    display(img)
            else:
                display(screenshot)

        return screenshot  # this will be a list of combine_screens == False

    def find(self, description, screenshot=None):
        if description.startswith('"') and description.endswith('"'):
            return self.find_text(description.strip('"'), screenshot)
        else:
            try:
                if self.computer.debug:
                    print("DEBUG MODE ON")
                    print("NUM HASHES:", len(self._hashes))
                else:
                    message = format_to_recipient(
                        "Locating this icon will take ~15 seconds. Subsequent icons should be found more quickly.",
                        recipient="user",
                    )
                    print(message)

                if len(self._hashes) > 5000:
                    self._hashes = dict(list(self._hashes.items())[-5000:])

                from .point.point import point

                result = point(
                    description, screenshot, self.computer.debug, self._hashes
                )

                return result
            except Exception:
                if self.computer.debug:
                    # We want to know these bugs lmao
                    raise
                if self.computer.offline:
                    raise
                message = format_to_recipient(
                    "Locating this icon will take ~30 seconds. We're working on speeding this up.",
                    recipient="user",
                )
                print(message)

                # Take a screenshot
                if screenshot == None:
                    screenshot = self.screenshot(show=False)

                # Downscale the screenshot to 1920x1080
                screenshot = screenshot.resize((1920, 1080))

                # Convert the screenshot to base64
                buffered = BytesIO()
                screenshot.save(buffered, format="PNG")
                screenshot_base64 = base64.b64encode(buffered.getvalue()).decode()

                try:
                    response = requests.post(
                        f'{self.computer.api_base.strip("/")}/point/',
                        json={"query": description, "base64": screenshot_base64},
                    )
                    return response.json()
                except Exception as e:
                    raise Exception(
                        str(e)
                        + "\n\nIcon locating API not available, or we were unable to find the icon. Please try another method to find this icon."
                    )

    def find_text(self, text, screenshot=None):
        """
        Searches for specified text within a screenshot or the current screen if no screenshot is provided.
        """
        if screenshot == None:
            screenshot = self.screenshot(show=False)

        if not self.computer.offline:
            # Convert the screenshot to base64
            buffered = BytesIO()
            screenshot.save(buffered, format="PNG")
            screenshot_base64 = base64.b64encode(buffered.getvalue()).decode()

            try:
                response = requests.post(
                    f'{self.computer.api_base.strip("/")}/point/text/',
                    json={"query": text, "base64": screenshot_base64},
                )
                response = response.json()
                return response
            except Exception:
                print("Attempting to find the text locally.")

        # We'll only get here if 1) self.computer.offline = True, or the API failed

        # Find the text in the screenshot
        centers = find_text_in_image(screenshot, text, self.computer.debug)

        return [
            {"coordinates": center, "text": "", "similarity": 1} for center in centers
        ]  # Have it deliver the text properly soon.

    def get_text_as_list_of_lists(self, screenshot=None):
        """
        Extracts and returns text from a screenshot or the current screen as a list of lists, each representing a line of text.
        """
        if screenshot == None:
            screenshot = self.screenshot(show=False, force_image=True)

        if not self.computer.offline:
            # Convert the screenshot to base64
            buffered = BytesIO()
            screenshot.save(buffered, format="PNG")
            screenshot_base64 = base64.b64encode(buffered.getvalue()).decode()

            try:
                response = requests.post(
                    f'{self.computer.api_base.strip("/")}/text/',
                    json={"base64": screenshot_base64},
                )
                response = response.json()
                return response
            except Exception:
                print("Attempting to get the text locally.")

        # We'll only get here if 1) self.computer.offline = True, or the API failed

        try:
            return pytesseract_get_text(screenshot)
        except Exception:
            raise Exception(
                "Failed to find text locally.\n\nTo find text in order to use the mouse, please make sure you've installed `pytesseract` along with the Tesseract executable (see this Stack Overflow answer for help installing Tesseract: https://stackoverflow.com/questions/50951955/pytesseract-tesseractnotfound-error-tesseract-is-not-installed-or-its-not-i)."
            )


def take_screenshot_to_pil(screen=0, combine_screens=True):
    # Get information about all screens
    monitors = screeninfo.get_monitors()
    if screen == -1:  # All screens
        # Take a screenshot of each screen and save them in a list
        screenshots = [
            pyautogui.screenshot(
                region=(monitor.x, monitor.y, monitor.width, monitor.height)
            )
            for monitor in monitors
        ]

        if combine_screens:
            # Combine all screenshots horizontally
            total_width = sum([img.width for img in screenshots])
            max_height = max([img.height for img in screenshots])

            # Create a new image with a size that can contain all screenshots
            new_img = Image.new("RGB", (total_width, max_height))

            # Paste each screenshot into the new image
            x_offset = 0
            for i, img in enumerate(screenshots):
                # Convert PIL Image to OpenCV Image (numpy array)
                img_cv = np.array(img)
                img_cv = cv2.cvtColor(img_cv, cv2.COLOR_RGB2BGR)

                # Convert new_img PIL Image to OpenCV Image (numpy array)
                new_img_cv = np.array(new_img)
                new_img_cv = cv2.cvtColor(new_img_cv, cv2.COLOR_RGB2BGR)

                # Paste each screenshot into the new image using OpenCV
                new_img_cv[
                    0 : img_cv.shape[0], x_offset : x_offset + img_cv.shape[1]
                ] = img_cv
                x_offset += img.width

                # Add monitor labels using OpenCV
                font = cv2.FONT_HERSHEY_SIMPLEX
                font_scale = 4
                font_color = (255, 255, 255)
                line_type = 2

                if i == 0:
                    text = "Primary Monitor"
                else:
                    text = f"Monitor {i}"

                # Calculate the font scale that will fit the text perfectly in the center of the monitor
                text_size = cv2.getTextSize(text, font, font_scale, line_type)[0]
                font_scale = min(img.width / text_size[0], img.height / text_size[1])

                # Recalculate the text size with the new font scale
                text_size = cv2.getTextSize(text, font, font_scale, line_type)[0]

                # Calculate the position to center the text
                text_x = x_offset - img.width // 2 - text_size[0] // 2
                text_y = max_height // 2 - text_size[1] // 2

                cv2.putText(
                    new_img_cv,
                    text,
                    (text_x, text_y),
                    font,
                    font_scale,
                    font_color,
                    line_type,
                )

                # Convert new_img from OpenCV Image back to PIL Image
                new_img_cv = cv2.cvtColor(new_img_cv, cv2.COLOR_BGR2RGB)
                new_img = Image.fromarray(new_img_cv)

            return new_img
        else:
            return screenshots
    elif screen > 0:
        # Take a screenshot of the selected screen
        return pyautogui.screenshot(
            region=(
                monitors[screen].x,
                monitors[screen].y,
                monitors[screen].width,
                monitors[screen].height,
            )
        )

    else:
        # Take a screenshot of the primary screen
        return pyautogui.screenshot(
            region=(
                monitors[screen].x,
                monitors[screen].y,
                monitors[screen].width,
                monitors[screen].height,
            )
        )


def get_displays():
    monitors = get_monitors()
    return monitors
