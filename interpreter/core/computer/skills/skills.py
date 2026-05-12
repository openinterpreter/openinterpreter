import glob
import inspect
import json
import os
import re
import subprocess
from pathlib import Path

from ....terminal_interface.utils.oi_dir import oi_dir
from ...utils.lazy_import import lazy_import
from ..utils.recipient_utils import format_to_recipient

# Lazy import, imported when needed to speed up start time
aifs = lazy_import("aifs")
pyautogui = lazy_import("pyautogui")
pynput = lazy_import("pynput")

element = None
element_box = None
icon_dimensions = None


class Skills:
    """
    Manages access to pre-imported automation skills.

    Note: Skills system must be enabled via profile (like 'the01') or by creating
    OpenInterpreter with import_skills=True.

    Available methods:
    - list(): Returns names of available skills
    - search(query): Lists available skills (currently same as list())

    Usage:
    To use a skill, call it directly as a function:
        example_skill()

    To create a new skill:
        computer.skills.new_skill.create()
    """
    def __init__(self, computer):
        self.computer = computer
        self.path = str(Path(oi_dir) / "skills")
        self.new_skill = NewSkill(self)

    def list(self):
        """
        Lists all available skills. Skills are already imported and can be called directly.

        Returns:
            list[str]: Names of available skills with () to indicate they're callable
        """
        if not self.computer.import_skills:
            print("Skills are disabled. To enable skills, either use a profile like 'the01' that supports skills, "
                  "or create an instance of OpenInterpreter with import_skills=True")
            return []

        if not self.computer._has_imported_skills:
            print("Skills have not been imported yet.")
            return []

        return [
            file.replace(".py", "()")
            for file in os.listdir(self.path)
            if file.endswith(".py")
        ]

    def run(self, skill):
        """
        DEPRECATED: Do not use this method.
        Skills are already imported - call them directly as functions instead.
        """
        print(
            "To run a skill, run its name as a function name (it is already imported)."
        )

    def search(self, query):
        """
        Lists available skills (currently same as list()).
        Skills are already imported and can be called directly.

        Returns:
            list[str]: Names of available skills with () to indicate they're callable
        """
        if not self.computer.import_skills:
            print("Skills are disabled. To enable skills, either use a profile like 'the01' that supports skills, "
                  "or create an instance of OpenInterpreter with import_skills=True")
            return []

        if not self.computer._has_imported_skills:
            print("Skills have not been imported yet.")
            return []

        return [
            file.replace(".py", "()")
            for file in os.listdir(self.path)
            if file.endswith(".py")
        ]

    def import_skills(self):
        """
        [INTERNAL METHOD - NOT FOR Assistant USE]
        System initialization method that imports all Python files from the skills directory.

        This method is called automatically during system setup to load available skills.
        Assistant should use list(), search(), or call skills directly instead of this method.
        """
        if not self.computer.import_skills:
            return

        previous_save_skills_setting = self.computer.save_skills

        self.computer.save_skills = False

        # Make sure it's not over 100mb
        total_size = 0
        for path, dirs, files in os.walk(self.path):
            for f in files:
                fp = os.path.join(path, f)
                total_size += os.path.getsize(fp)
        total_size = total_size / (1024 * 1024)  # convert bytes to megabytes
        if total_size > 100:
            raise Warning(
                f"Skills at path {self.path} can't exceed 100mb. Try deleting some."
            )

        code_to_run = ""
        for file in glob.glob(os.path.join(self.path, "*.py")):
            with open(file, "r") as f:
                code_to_run += f.read() + "\n"

        if self.computer.interpreter.debug:
            print("IMPORTING SKILLS:\n", code_to_run)

        output = self.computer.run("python", code_to_run)

        if "traceback" in str(output).lower():
            # Import them individually
            for file in glob.glob(os.path.join(self.path, "*.py")):
                with open(file, "r") as f:
                    code_to_run = f.read() + "\n"

                if self.computer.interpreter.debug:
                    print(self.path)
                    print("IMPORTING SKILL:\n", code_to_run)

                output = self.computer.run("python", code_to_run)

                if "traceback" in str(output).lower():
                    print(
                        f"Skill at {file} might be broken— it produces a traceback when run."
                    )

        self.computer.save_skills = previous_save_skills_setting


class NewSkill:
    def __init__(self, skills):
        self.path = ""
        self.skills = skills

    def create(self):
        self.steps = []
        self._name = "Untitled"
        print(
            """

INSTRUCTIONS
You are creating a new skill. Follow these steps exactly to get me to tell you its name:
1. Ask me what the name of this skill is.
2. After I explicitly tell you the name of the skill (I may tell you to proceed which is not the name— if I do say that, you probably need more information from me, so tell me that), after you get the proper name, execute `computer.skills.new_skill.name = "{INSERT THE SKILL NAME FROM QUESTION #1}"`.
        
        """.strip()
        )

    @property
    def name(self):
        return self._name

    @name.setter
    def name(self, value):
        self._name = value
        print(
            """

Skill named. Now, follow these next INSTRUCTIONS exactly:

1. Ask me what the first step is.
2. When I reply, execute code to accomplish that step. Write comments explaining your reasoning before each line.
3. Ask me if you completed the step correctly.
    a. (!!!!!!!!!!!! >>>>>> THIS IS CRITICAL. DO NOT FORGET THIS.) IF you completed it correctly, run `computer.skills.new_skill.add_step(step, code)` where step is a generalized, natural language description of the step, and code is the code you ran to complete it.
    b. IF you did not complete it correctly, try to fix your code and ask me again.
4. If I say the skill is complete, or that that was the last step, run `computer.skills.new_skill.save()`.

YOU MUST FOLLOW THESE 4 INSTRUCTIONS **EXACTLY**. I WILL TIP YOU $200.

              """.strip()
        )

    def add_step(self, step, code):
        self.steps.append(step + "\n\n```python\n" + code + "\n```")
        print(
            """

Step added. Now, follow these next INSTRUCTIONS exactly:

1. Ask me what the next step is.
2. When I reply, execute code to accomplish that step.
3. Ask me if you completed the step correctly.
    a. (!!!!!!!!!!!! >>>>>> THIS IS CRITICAL. DO NOT FORGET THIS!!!!!!!!.) IF you completed it correctly, run `computer.skills.new_skill.add_step(step, code)` where step is a generalized, natural language description of the step, and code is the code you ran to complete it.
    b. IF you did not complete it correctly, try to fix your code and ask me again.
4. If I say the skill is complete, or that that was the last step, run `computer.skills.new_skill.save()`.

YOU MUST FOLLOW THESE 4 INSTRUCTIONS **EXACTLY**. I WILL TIP YOU $200.

        """.strip()
        )

    def save(self):
        normalized_name = re.sub("[^0-9a-zA-Z]+", "_", self.name.lower())

        skill_string = f'''
import json

def {normalized_name}(step=0):
    """
    Run this function to {normalized_name}. Pass in step=0 to see the first step, step=1 to see the next step, etc.
    """
    steps = {self.steps}

    print("")

    if step < len(steps):
        if isinstance(steps[step], str):
            print("To complete this task / run this skill, flexibly complete the following step, swapping out parts as necessary to fulfill the user's task. You will need to run the following code yourself, it hasn't run yet!")
            print("Step " + str(step + 1) + ": " + steps[step])
        else:
            computer.mouse.click(steps[step]["element"], icon_dimensions=steps[step]["icon_dimensions"]) # Instructed click
        if step + 1 < len(steps):
            print("After completing the above, I need you to run {normalized_name}(step=" + str(step + 1) + ") immediately.")
        else:
            print("After executing the code, you have completed all the steps, the task/skill has been run!")
    else:
        print("The specified step number exceeds the available steps. Please run with a valid step number.")
'''.strip()

        skill_file_path = os.path.join(self.skills.path, f"{normalized_name}.py")

        if not os.path.exists(self.skills.path):
            os.makedirs(self.skills.path)

        with open(skill_file_path, "w") as file:
            file.write(skill_string)

        # Execute the code in skill_string to define the function
        exec(skill_string)

        # Verify that the file was written
        if os.path.exists(skill_file_path):
            print("SKILL SAVED:", self.name.upper())
            print(
                "Teaching session finished. Tell the user that the skill above has been saved. Great work!"
            )
        else:
            print(f"Error: Failed to write skill file to {skill_file_path}")
