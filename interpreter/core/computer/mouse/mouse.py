import time
import warnings

from IPython.display import display
from PIL import Image

from ...utils.lazy_import import lazy_import
from ..utils.recipient_utils import format_to_recipient

# Lazy import of optional packages
try:
    cv2 = lazy_import("cv2")
except Exception:
    cv2 = None  # Fixes colab error
np = lazy_import("numpy")
pyautogui = lazy_import("pyautogui")
plt = lazy_import("matplotlib.pyplot")


class Mouse:
    def __init__(self, computer):
        self.computer = computer

    def scroll(self, clicks):
        """
        Scrolls the mouse wheel up or down the specified number of clicks.
        """
        pyautogui.scroll(clicks)

    def position(self):
        """
        Get the current mouse position.

        Returns:
            tuple: A tuple (x, y) representing the mouse's current position on the screen.
        """
        try:
            return pyautogui.position()
        except Exception as e:
            raise RuntimeError(
                f"An error occurred while retrieving the mouse position: {e}. "
            )

    def move(self, *args, x=None, y=None, icon=None, text=None, screenshot=None):
        """
        Moves the mouse to specified coordinates, an icon, or text.
        """
        if len(args) > 1:
            raise ValueError(
                "Too many positional arguments provided. To move/click specific coordinates, use kwargs (x=x, y=y).\n\nPlease take a screenshot with computer.display.view() to find text/icons to click, then use computer.mouse.click(text) or computer.mouse.click(icon=description_of_icon) if at all possible. This is **significantly** more accurate than using coordinates. Specifying (x=x, y=y) is highly likely to fail. Specifying ('text to click') is highly likely to succeed."
            )
        elif len(args) == 1 or text != None:
            if len(args) == 1:
                text = args[0]

            if screenshot == None:
                screenshot = self.computer.display.screenshot(show=False)

            coordinates = self.computer.display.find(
                '"' + text + '"', screenshot=screenshot
            )

            is_fuzzy = any([c["similarity"] != 1 for c in coordinates])
            # nah just hey, if it's fuzzy, then whatever, it prob wont see the message then decide something else (not really smart enough yet usually)
            # so for now, just lets say it's always not fuzzy so if there's 1 coord it will pick it automatically
            is_fuzzy = False

            if len(coordinates) == 0:
                return self.move(icon=text)  # Is this a better solution?

                if self.computer.emit_images:
                    plt.imshow(np.array(screenshot))
                    with warnings.catch_warnings():
                        warnings.simplefilter("ignore")
                        plt.show()
                raise ValueError(
                    f"@@@HIDE_TRACEBACK@@@Your text ('{text}') was not found on the screen. Please try again. If you're 100% sure the text should be there, consider using `computer.mouse.scroll(-10)` to scroll down.\n\nYou can use `computer.display.get_text_as_list_of_lists()` to see all the text on the screen."
                )
            elif len(coordinates) > 1 or is_fuzzy:
                if self.computer.emit_images:
                    # Convert the screenshot to a numpy array for drawing
                    img_array = np.array(screenshot)
                    gray = cv2.cvtColor(img_array, cv2.COLOR_BGR2GRAY)
                    img_draw = cv2.cvtColor(gray, cv2.COLOR_GRAY2RGB)

                    # Iterate over the response items
                    for i, item in enumerate(coordinates):
                        width, height = screenshot.size
                        x, y = item["coordinates"]
                        x *= width
                        y *= height

                        x = int(x)
                        y = int(y)

                        # Draw a solid blue circle around the found text
                        cv2.circle(img_draw, (x, y), 20, (0, 0, 255), -1)
                        # Put the index number in the center of the circle in white
                        cv2.putText(
                            img_draw,
                            str(i),
                            (x - 10, y + 10),
                            cv2.FONT_HERSHEY_SIMPLEX,
                            1,
                            (255, 255, 255),
                            2,
                            cv2.LINE_AA,
                        )

                    img_pil = Image.fromarray(img_draw)
                    display(img_pil)

                coordinates = [
                    f"{i}: ({int(item['coordinates'][0]*self.computer.display.width)}, {int(item['coordinates'][1]*self.computer.display.height)}) "
                    + '"'
                    + item["text"]
                    + '"'
                    for i, item in enumerate(coordinates)
                ]
                if is_fuzzy:
                    error_message = (
                        f"@@@HIDE_TRACEBACK@@@Your text ('{text}') was not found exactly, but some similar text was found. Please review the attached image, then click/move over one of the following coordinates with computer.mouse.click(x=x, y=y) or computer.mouse.move(x=x, y=y):\n"
                        + "\n".join(coordinates)
                    )
                else:
                    error_message = (
                        f"@@@HIDE_TRACEBACK@@@Your text ('{text}') was found multiple times on the screen. Please review the attached image, then click/move over one of the following coordinates with computer.mouse.click(x=x, y=y) or computer.mouse.move(x=x, y=y):\n"
                        + "\n".join(coordinates)
                    )
                raise ValueError(error_message)
            else:
                x, y = coordinates[0]["coordinates"]
                x *= self.computer.display.width
                y *= self.computer.display.height
                x = int(x)
                y = int(y)

        elif x is not None and y is not None:
            print(
                format_to_recipient(
                    "Unless you have just received these EXACT coordinates from a computer.mouse.move or computer.mouse.click command, PLEASE take a screenshot with computer.display.view() to find TEXT OR ICONS to click, then use computer.mouse.click(text) or computer.mouse.click(icon=description_of_icon) if at all possible. This is **significantly** more accurate than using coordinates. Specifying (x=x, y=y) is highly likely to fail. Specifying ('text to click') is highly likely to succeed.",
                    "assistant",
                )
            )
        elif icon is not None:
            if screenshot == None:
                screenshot = self.computer.display.screenshot(show=False)

            coordinates = self.computer.display.find(icon.strip('"'), screenshot)

            if len(coordinates) > 1:
                if self.computer.emit_images:
                    # Convert the screenshot to a numpy array for drawing
                    img_array = np.array(screenshot)
                    gray = cv2.cvtColor(img_array, cv2.COLOR_BGR2GRAY)
                    img_draw = cv2.cvtColor(gray, cv2.COLOR_GRAY2RGB)

                    # Iterate over the response items
                    for i, item in enumerate(coordinates):
                        width, height = screenshot.size
                        x, y = item
                        x *= width
                        y *= height

                        x = int(x)
                        y = int(y)

                        # Draw a solid blue circle around the found text
                        cv2.circle(img_draw, (x, y), 20, (0, 0, 255), -1)
                        # Put the index number in the center of the circle in white
                        cv2.putText(
                            img_draw,
                            str(i),
                            (x - 10, y + 10),
                            cv2.FONT_HERSHEY_SIMPLEX,
                            1,
                            (255, 255, 255),
                            2,
                            cv2.LINE_AA,
                        )

                    plt.imshow(img_draw)
                    with warnings.catch_warnings():
                        warnings.simplefilter("ignore")
                        plt.show()

                coordinates = [
                    f"{i}: {int(item[0]*self.computer.display.width)}, {int(item[1]*self.computer.display.height)}"
                    for i, item in enumerate(coordinates)
                ]
                error_message = (
                    f"Your icon ('{text}') was found multiple times on the screen. Please click one of the following coordinates with computer.mouse.move(x=x, y=y):\n"
                    + "\n".join(coordinates)
                )
                raise ValueError(error_message)
            else:
                x, y = coordinates[0]
                x *= self.computer.display.width
                y *= self.computer.display.height
                x = int(x)
                y = int(y)

        else:
            raise ValueError("Either text, icon, or both x and y must be provided")

        if self.computer.verbose:
            if not screenshot:
                screenshot = self.computer.display.screenshot(show=False)

            # Convert the screenshot to a numpy array for drawing
            img_array = np.array(screenshot)
            gray = cv2.cvtColor(img_array, cv2.COLOR_BGR2GRAY)
            img_draw = cv2.cvtColor(gray, cv2.COLOR_GRAY2RGB)

            # Scale drawing_x and drawing_y from screen size to screenshot size for drawing purposes
            drawing_x = int(x * screenshot.width / self.computer.display.width)
            drawing_y = int(y * screenshot.height / self.computer.display.height)

            # Draw a solid blue circle around the place we're clicking
            cv2.circle(img_draw, (drawing_x, drawing_y), 20, (0, 0, 255), -1)

            plt.imshow(img_draw)
            with warnings.catch_warnings():
                warnings.simplefilter("ignore")
                plt.show()

        # pyautogui.moveTo(x, y, duration=0.5)
        smooth_move_to(x, y)

    def click(self, *args, button="left", clicks=1, interval=0.1, **kwargs):
        """
        Clicks the mouse at the specified coordinates, icon, or text.
        """
        if args or kwargs:
            self.move(*args, **kwargs)
        pyautogui.click(button=button, clicks=clicks, interval=interval)

    def double_click(self, *args, button="left", interval=0.1, **kwargs):
        """
        Double-clicks the mouse at the specified coordinates, icon, or text.
        """
        if args or kwargs:
            self.move(*args, **kwargs)
        pyautogui.doubleClick(button=button, interval=interval)

    def triple_click(self, *args, button="left", interval=0.1, **kwargs):
        """
        Triple-clicks the mouse at the specified coordinates, icon, or text.
        """
        if args or kwargs:
            self.move(*args, **kwargs)
        pyautogui.tripleClick(button=button, interval=interval)

    def right_click(self, *args, **kwargs):
        """
        Right-clicks the mouse at the specified coordinates, icon, or text.
        """
        if args or kwargs:
            self.move(*args, **kwargs)
        pyautogui.rightClick()

    def down(self):
        """
        Presses the mouse button down.
        """
        pyautogui.mouseDown()

    def up(self):
        """
        Releases the mouse button.
        """
        pyautogui.mouseUp()


import math
import time


def smooth_move_to(x, y, duration=2):
    start_x, start_y = pyautogui.position()
    dx = x - start_x
    dy = y - start_y
    distance = math.hypot(dx, dy)  # Calculate the distance in pixels

    start_time = time.time()

    while True:
        elapsed_time = time.time() - start_time
        if elapsed_time > duration:
            break

        t = elapsed_time / duration
        eased_t = (1 - math.cos(t * math.pi)) / 2  # easeInOutSine function

        target_x = start_x + dx * eased_t
        target_y = start_y + dy * eased_t
        pyautogui.moveTo(target_x, target_y)

    # Ensure the mouse ends up exactly at the target (x, y)
    pyautogui.moveTo(x, y)
