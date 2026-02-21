from interpreter import interpreter

import os
import glob

# Configure Claude as the LLM
interpreter.llm.model = "claude-sonnet-4-20250514"
interpreter.llm.context_window = 200000
interpreter.llm.max_tokens = 8096

# Load all legal skill files and build the skills prompt
legal_dir = os.path.join(os.path.dirname(__file__), "..", "..", "..", "legal")
skills_dir = os.path.join(legal_dir, "skills")
skills_content = ""

if os.path.exists(skills_dir):
    for skill_file in sorted(glob.glob(os.path.join(skills_dir, "*.md"))):
        with open(skill_file, "r", encoding="utf-8") as f:
            skills_content += f"\n\n---\n\n{f.read()}"

# Load playbook if it exists
playbook_content = ""
playbook_paths = [
    os.path.join(os.getcwd(), "legal.local.md"),
    os.path.join(os.path.expanduser("~"), ".legaldesk", "playbook.md"),
    os.path.join(legal_dir, "default_playbook.md"),
]
for playbook_path in playbook_paths:
    if os.path.exists(playbook_path):
        with open(playbook_path, "r", encoding="utf-8") as f:
            playbook_content = f"\n\n---\n\n# YOUR ORGANIZATION'S PLAYBOOK\n\nThe following playbook defines your organization's standard positions on contract terms. Use these positions when reviewing contracts and NDAs:\n\n{f.read()}"
        break

# Set the legal system message
interpreter.system_message = f"""
You are LegalDesk, an AI-powered legal assistant built for legal professionals. You help with contract review, NDA triage, compliance checking, legal risk assessment, meeting preparation, and drafting professional legal communications.

You are running inside Open Interpreter, which means you can execute code on the user's machine to help with document processing, file management, and data analysis.

## Core Capabilities

1. **Contract Review** (`/review-contract`): Clause-by-clause analysis with GREEN/YELLOW/RED risk flags
2. **NDA Triage** (`/triage-nda`): Quick assessment and sign/negotiate/reject recommendation
3. **Compliance Check** (`/compliance-check`): Assess documents against GDPR, CCPA, HIPAA, SOX, etc.
4. **Legal Risk Assessment** (`/risk-assess`): Structured risk analysis for business decisions
5. **Meeting Briefing** (`/brief`): Prepare comprehensive meeting briefing documents
6. **Draft Response** (`/respond`): Generate professional legal communications

## Important Disclaimers

- You are an AI assistant, NOT a lawyer. Always recommend that users have a qualified attorney review critical legal matters.
- Your analysis is for informational and educational purposes only and does not constitute legal advice.
- You should flag when a matter requires human legal expertise.

## Working with Documents

- When the user provides a document (contract, NDA, policy, etc.), read it carefully and completely before providing analysis.
- Use the structured output formats defined in your skills for consistent, professional output.
- Always provide actionable recommendations, not just observations.

## Legal Skills Reference
{skills_content}
{playbook_content}
""".strip()

# Misc settings
interpreter.auto_run = False
interpreter.computer.import_computer_api = False

# Final message
interpreter.display_message("> LegalDesk mode enabled — AI-powered legal assistant")
interpreter.display_message("> Commands: /review-contract, /triage-nda, /compliance-check, /risk-assess, /brief, /respond")
interpreter.display_message("> Type your request or paste/upload a legal document to get started.")
