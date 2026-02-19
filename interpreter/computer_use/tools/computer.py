import asyncio
import base64
import math
import platform
import subprocess
import tempfile
import time
from enum import StrEnum
from pathlib import Path
from typing import Literal, TypedDict
from uuid import uuid4

# Add import for PyAutoGUI
import pyautogui
from anthropic.types.beta import BetaToolComputerUse20241022Param

from .base import BaseAnthropicTool, ToolError, ToolResult
from .run import run

OUTPUT_DIR = "/tmp/outputs"

TYPING_DELAY_MS = 12
TYPING_GROUP_SIZE = 50

Action = Literal[
    "key",
    "type",
    "mouse_move",
    "left_click",
    "left_click_drag",
    "right_click",
    "middle_click",
    "double_click",
    "screenshot",
    "cursor_position",
]


class Resolution(TypedDict):
    width: int
    height: int


# sizes above XGA/WXGA are not recommended (see README.md)
# scale down to one of these targets if ComputerTool._scaling_enabled is set
MAX_SCALING_TARGETS: dict[str, Resolution] = {
    "XGA": Resolution(width=1024, height=768),  # 4:3
    "WXGA": Resolution(width=1280, height=800),  # 16:10
    "FWXGA": Resolution(width=1366, height=768),  # ~16:9
}


class ScalingSource(StrEnum):
    COMPUTER = "computer"
    API = "api"


class ComputerToolOptions(TypedDict):
    display_height_px: int
    display_width_px: int
    display_number: int | None


def chunks(s: str, chunk_size: int) -> list[str]:
    return [s[i : i + chunk_size] for i in range(0, len(s), chunk_size)]


MACOS_MODIFIER_ALIASES = {
    "alt": "option",
    "cmd": "command",
    "command": "command",
    "control": "control",
    "ctrl": "control",
    "option": "option",
    "shift": "shift",
    "super": "command",
}


def _escape_applescript_string(value: str) -> str:
    return value.replace("\\", "\\\\").replace('"', '\\"')


def _normalize_macos_modifier(modifier: str) -> str | None:
    return MACOS_MODIFIER_ALIASES.get(modifier.strip().lower())


def _run_macos_hotkey(keystroke: str, modifiers: list[str]) -> bool:
    applescript_modifiers = []
    for modifier in modifiers:
        normalized_modifier = _normalize_macos_modifier(modifier)
        if not normalized_modifier:
            return False
        applescript_modifiers.append(f"{normalized_modifier} down")

    if not applescript_modifiers:
        return False

    escaped_keystroke = _escape_applescript_string(keystroke)
    using_clause = applescript_modifiers[0]
    if len(applescript_modifiers) > 1:
        using_clause = "{" + ", ".join(applescript_modifiers) + "}"

    script = f"""
    tell application "System Events"
        keystroke "{escaped_keystroke}" using {using_clause}
    end tell
    """

    try:
        subprocess.run(["osascript", "-e", script], check=False)
        return True
    except OSError:
        return False


def smooth_move_to(x, y, duration=1.2):
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


class ComputerTool(BaseAnthropicTool):
    """
    A tool that allows the agent to interact with the primary monitor's screen, keyboard, and mouse.
    The tool parameters are defined by Anthropic and are not editable.
    """

    name: Literal["computer"] = "computer"
    api_type: Literal["computer_20241022"] = "computer_20241022"
    width: int
    height: int
    display_num: None  # Simplified to always be None since we're only using primary display

    _screenshot_delay = 2.0
    _scaling_enabled = True

    @property
    def options(self) -> ComputerToolOptions:
        width, height = self.scale_coordinates(
            ScalingSource.COMPUTER, self.width, self.height
        )
        return {
            "display_width_px": width,
            "display_height_px": height,
            "display_number": self.display_num,
        }

    def to_params(self) -> BetaToolComputerUse20241022Param:
        return {"name": self.name, "type": self.api_type, **self.options}

    def __init__(self):
        super().__init__()
        self.width, self.height = pyautogui.size()
        self.display_num = None

    async def __call__(
        self,
        *,
        action: Action,
        text: str | None = None,
        coordinate: tuple[int, int] | None = None,
        **kwargs,
    ):
        if action in ("mouse_move", "left_click_drag"):
            if coordinate is None:
                raise ToolError(f"coordinate is required for {action}")
            x, y = self.scale_coordinates(
                ScalingSource.API, coordinate[0], coordinate[1]
            )

            if action == "mouse_move":
                smooth_move_to(x, y)
            elif action == "left_click_drag":
                smooth_move_to(x, y)
                pyautogui.dragTo(x, y, button="left")

        elif action in ("key", "type"):
            if text is None:
                raise ToolError(f"text is required for {action}")

            if action == "key":
                if platform.system() == "Darwin":  # Check if we're on macOS
                    text = text.replace("super+", "command+")

                # Normalize key names
                def normalize_key(key):
                    key = key.lower().replace("_", "")
                    key_map = {
                        "pagedown": "pgdn",
                        "pageup": "pgup",
                        "enter": "return",
                        "return": "enter",
                        # Add more mappings as needed
                    }
                    return key_map.get(key, key)

                keys = [normalize_key(k) for k in text.split("+")]

                if len(keys) > 1:
                    if "darwin" in platform.system().lower():
                        # Use AppleScript for hotkey on macOS
                        keystroke = keys[-1]
                        modifiers = keys[:-1]
                        if keystroke.lower() == "space":
                            keystroke = " "
                        elif keystroke.lower() in {"enter", "return"}:
                            keystroke = "\n"
                        if not _run_macos_hotkey(keystroke, modifiers):
                            pyautogui.hotkey(*keys)
                    else:
                        pyautogui.hotkey(*keys)
                else:
                    pyautogui.press(keys[0])
            elif action == "type":
                pyautogui.write(text, interval=TYPING_DELAY_MS / 1000)

        elif action in ("left_click", "right_click", "double_click", "middle_click"):
            time.sleep(0.1)
            button = {
                "left_click": "left",
                "right_click": "right",
                "middle_click": "middle",
            }
            if action == "double_click":
                pyautogui.click()
                time.sleep(0.1)
                pyautogui.click()
            else:
                pyautogui.click(button=button.get(action, "left"))

        elif action == "screenshot":
            return await self.screenshot()

        elif action == "cursor_position":
            x, y = pyautogui.position()
            x, y = self.scale_coordinates(ScalingSource.COMPUTER, x, y)
            return ToolResult(output=f"X={x},Y={y}")

        else:
            raise ToolError(f"Invalid action: {action}")

        # Take a screenshot after the action (except for cursor_position)
        if action != "cursor_position":
            return await self.screenshot()

    async def screenshot(self):
        """Take a screenshot of the current screen and return the base64 encoded image."""
        temp_dir = Path(tempfile.gettempdir())
        path = temp_dir / f"screenshot_{uuid4().hex}.png"

        screenshot = pyautogui.screenshot()
        screenshot.save(str(path))

        if self._scaling_enabled:
            x, y = self.scale_coordinates(
                ScalingSource.COMPUTER, self.width, self.height
            )
            # Use PIL directly instead of shell convert command
            from PIL import Image

            with Image.open(path) as img:
                img = img.resize((x, y), Image.Resampling.LANCZOS)
                img.save(path)

        if path.exists():
            base64_image = base64.b64encode(path.read_bytes()).decode()
            path.unlink()  # Remove the temporary file
            return ToolResult(base64_image=base64_image)
        raise ToolError(f"Failed to take screenshot")

    async def shell(self, command: str, take_screenshot=True) -> ToolResult:
        """Run a shell command and return the output, error, and optionally a screenshot."""
        _, stdout, stderr = await run(command)
        base64_image = None

        if take_screenshot:
            # delay to let things settle before taking a screenshot
            await asyncio.sleep(self._screenshot_delay)
            base64_image = (await self.screenshot()).base64_image

        return ToolResult(output=stdout, error=stderr, base64_image=base64_image)

    def scale_coordinates(self, source: ScalingSource, x: int, y: int):
        """Scale coordinates to a target maximum resolution."""
        if not self._scaling_enabled:
            return x, y
        ratio = self.width / self.height
        target_dimension = None
        for dimension in MAX_SCALING_TARGETS.values():
            # allow some error in the aspect ratio - not ratios are exactly 16:9
            if abs(dimension["width"] / dimension["height"] - ratio) < 0.02:
                if dimension["width"] < self.width:
                    target_dimension = dimension
                break
        if target_dimension is None:
            return x, y
        # should be less than 1
        x_scaling_factor = target_dimension["width"] / self.width
        y_scaling_factor = target_dimension["height"] / self.height
        if source == ScalingSource.API:
            if x > self.width or y > self.height:
                raise ToolError(f"Coordinates {x}, {y} are out of bounds")
            # scale up
            return round(x / x_scaling_factor), round(y / y_scaling_factor)
        # scale down
        return round(x * x_scaling_factor), round(y * y_scaling_factor)
