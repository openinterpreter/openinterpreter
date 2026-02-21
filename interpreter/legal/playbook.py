"""
Playbook system for LegalDesk.

Loads and manages legal playbooks that define an organization's standard
positions on contract terms, risk tolerances, and compliance requirements.
"""

import os

DEFAULT_PLAYBOOK_FILENAME = "legal.local.md"
PLAYBOOK_SEARCH_PATHS = [
    os.path.join(os.getcwd(), DEFAULT_PLAYBOOK_FILENAME),
    os.path.join(os.path.expanduser("~"), ".legaldesk", "playbook.md"),
]


def find_playbook():
    """Find the first available playbook file."""
    for path in PLAYBOOK_SEARCH_PATHS:
        if os.path.exists(path):
            return path
    return None


def load_playbook(path=None):
    """Load a playbook from file. Returns the content as a string, or None."""
    if path is None:
        path = find_playbook()
    if path is None:
        return None
    with open(path, "r", encoding="utf-8") as f:
        return f.read()


def save_playbook(content, path=None):
    """Save playbook content to file."""
    if path is None:
        path = os.path.join(os.getcwd(), DEFAULT_PLAYBOOK_FILENAME)
    os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        f.write(content)
    return path


def get_default_playbook_template():
    """Return a default playbook template for users to customize."""
    return """# Legal Playbook

> This is your organization's legal playbook. Customize the positions below
> to reflect your standard terms and risk tolerance. LegalDesk will use these
> positions when reviewing contracts and NDAs.

## Organization Info

- **Company Name**: [Your Company Name]
- **Entity Type**: [Corporation / LLC / Partnership]
- **Headquarters Jurisdiction**: [State/Country]
- **Preferred Governing Law**: [State/Country]
- **Industry**: [Your Industry]

## Contract Review Positions

### Limitation of Liability
- **Our Position**: Mutual cap at 12 months of fees paid/payable
- **Acceptable Range**: 6-24 months of fees
- **Reject If**: Unlimited liability or cap less than 3 months of fees
- **Carve-outs**: Uncapped for IP infringement, confidentiality breach, willful misconduct

### Indemnification
- **Our Position**: Mutual indemnification for IP infringement and breach of confidentiality
- **Acceptable Range**: Reasonable mutual indemnification with notice and control provisions
- **Reject If**: One-sided indemnification without caps or unreasonable scope

### Intellectual Property
- **Our Position**: Each party retains its pre-existing IP. We own all work product.
- **Acceptable Range**: Clear ownership with appropriate licenses back
- **Reject If**: Vendor claims ownership of our data or custom deliverables

### Data Protection
- **Our Position**: Full GDPR and CCPA compliance required. DPA must be in place.
- **Acceptable Range**: Compliance with applicable privacy laws
- **Reject If**: No data protection commitments or refusal to sign DPA

### Termination
- **Our Position**: Either party may terminate for convenience with 30 days notice
- **Acceptable Range**: 30-90 days notice for convenience termination
- **Reject If**: No termination for convenience or lock-in period exceeding 12 months

### Payment Terms
- **Our Position**: Net 30 from invoice date
- **Acceptable Range**: Net 15 to Net 60
- **Reject If**: Payment on order or less than Net 15

### Governing Law & Jurisdiction
- **Our Position**: [Your preferred jurisdiction]
- **Acceptable Range**: Major commercial jurisdictions (NY, DE, CA, London, Singapore)
- **Reject If**: Unfamiliar or unfavorable jurisdiction

### Confidentiality
- **Our Position**: Mutual obligations, 3-year term, standard exclusions
- **Acceptable Range**: 2-5 year term with all standard exclusions
- **Reject If**: Perpetual obligations or missing standard exclusions

### Non-Compete / Non-Solicitation
- **Our Position**: Not acceptable in vendor/service contracts
- **Acceptable Range**: Narrow non-solicitation of directly assigned personnel, 12 months max
- **Reject If**: Broad non-compete or non-solicitation exceeding 12 months

### Insurance
- **Our Position**: Standard commercial insurance requirements
- **Acceptable Range**: Reasonable insurance based on contract scope
- **Reject If**: Excessive insurance requirements disproportionate to contract value

### Force Majeure
- **Our Position**: Standard force majeure with pandemic/epidemic included
- **Acceptable Range**: Broad force majeure covering typical events
- **Reject If**: No force majeure clause or exclusion of pandemic events

## NDA Positions

### Type
- **Our Preference**: Mutual NDA
- **Accept One-Way**: Only when we are the disclosing party

### Term
- **Our Position**: 3-year confidentiality period
- **Acceptable Range**: 2-5 years
- **Reject If**: Perpetual or less than 1 year

### Scope
- **Our Position**: Limited to information disclosed for the stated purpose
- **Reject If**: Overly broad "all information" definition without exclusions

### Non-Solicitation in NDA
- **Our Position**: Not acceptable in NDAs
- **Reject If**: Non-solicitation or non-compete provisions in an NDA

## Risk Tolerance

- **Overall Risk Appetite**: Moderate
- **Maximum Acceptable Contract Value Without Legal Review**: $50,000
- **Require Board Approval Above**: $500,000
- **Auto-Reject Threshold**: Any contract with unlimited liability exposure
"""
