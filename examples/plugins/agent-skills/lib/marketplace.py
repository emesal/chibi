"""Marketplace operations for installing/managing skills."""
import json
import subprocess
import shutil
from pathlib import Path
from typing import List, Dict, Any

# Default marketplace sources
MARKETPLACE_SOURCES = [
    "https://github.com/anthropics/skills",
]

def install_skill(skill_ref: str, skills_dir: Path) -> str:
    """
    Install a skill from the marketplace.

    skill_ref format: "owner/skill-name" or full GitHub URL
    """
    skills_dir.mkdir(parents=True, exist_ok=True)

    # Parse skill reference
    if skill_ref.startswith("http"):
        repo_url = skill_ref
        skill_name = skill_ref.rstrip("/").split("/")[-1]
    elif "/" in skill_ref:
        owner, skill_name = skill_ref.split("/", 1)
        repo_url = f"https://github.com/{owner}/skills"
    else:
        return f"Error: Invalid skill reference '{skill_ref}'. Use 'owner/skill-name' format."

    target_dir = skills_dir / skill_name

    if target_dir.exists():
        return f"Skill '{skill_name}' is already installed. Remove it first to reinstall."

    # Try to fetch from GitHub
    try:
        # Clone sparse checkout of just the skill directory
        temp_dir = skills_dir / f".tmp_{skill_name}"

        result = subprocess.run(
            ["git", "clone", "--depth", "1", "--filter=blob:none", "--sparse", repo_url, str(temp_dir)],
            capture_output=True,
            text=True
        )

        if result.returncode != 0:
            return f"Error cloning repository: {result.stderr}"

        # Set up sparse checkout for the skill
        subprocess.run(
            ["git", "-C", str(temp_dir), "sparse-checkout", "set", f"skills/{skill_name}"],
            capture_output=True
        )

        # Move the skill to the target location
        skill_source = temp_dir / "skills" / skill_name
        if skill_source.exists():
            shutil.move(str(skill_source), str(target_dir))
            shutil.rmtree(str(temp_dir))
            return f"Successfully installed skill '{skill_name}'."
        else:
            shutil.rmtree(str(temp_dir))
            return f"Error: Skill '{skill_name}' not found in repository."

    except Exception as e:
        return f"Error installing skill: {e}"

def remove_skill(skill_ref: str, skills_dir: Path) -> str:
    """Remove an installed skill."""
    skill_name = skill_ref.split("/")[-1] if "/" in skill_ref else skill_ref
    target_dir = skills_dir / skill_name

    if not target_dir.exists():
        return f"Skill '{skill_name}' is not installed."

    try:
        shutil.rmtree(str(target_dir))
        return f"Successfully removed skill '{skill_name}'."
    except Exception as e:
        return f"Error removing skill: {e}"

def search_skills(query: str) -> List[Dict[str, Any]]:
    """Search for skills in the marketplace."""
    return [
        {"message": "Marketplace search not yet implemented. Check https://github.com/anthropics/skills for available skills."}
    ]

def list_available() -> List[Dict[str, Any]]:
    """List available skills from marketplace sources."""
    return [
        {"message": "Marketplace listing not yet implemented. Check https://github.com/anthropics/skills for available skills."}
    ]
