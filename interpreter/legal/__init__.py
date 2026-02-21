"""
LegalDesk - Legal AI module for Open Interpreter.

Provides legal-specific skills, document parsing, playbook management,
and slash commands for contract review, NDA triage, compliance checking,
and more.
"""

import os

LEGAL_DIR = os.path.dirname(os.path.abspath(__file__))
SKILLS_DIR = os.path.join(LEGAL_DIR, "skills")
TEMPLATES_DIR = os.path.join(LEGAL_DIR, "templates")
