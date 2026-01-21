"""Manage active skill state for allowed-tools enforcement."""
import json
from pathlib import Path
from typing import Optional, Dict, Any

# State file location (inside plugin directory)
STATE_FILE = Path(__file__).parent.parent / ".active_skill.json"

def get_active_skill() -> Optional[Dict[str, Any]]:
    """Get the currently active skill, if any."""
    if not STATE_FILE.exists():
        return None

    try:
        data = json.loads(STATE_FILE.read_text())
        return data
    except (json.JSONDecodeError, IOError):
        return None

def set_active_skill(name: str, allowed_tools: Optional[str]):
    """Set the active skill."""
    data = {
        "name": name,
        "allowed_tools": allowed_tools
    }
    STATE_FILE.write_text(json.dumps(data))

def clear_active_skill():
    """Clear the active skill state."""
    if STATE_FILE.exists():
        STATE_FILE.unlink()
