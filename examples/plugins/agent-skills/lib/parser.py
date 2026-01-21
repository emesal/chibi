"""Parse SKILL.md files according to Agent Skills specification."""
import re
from dataclasses import dataclass
from pathlib import Path
from typing import Optional, List
import yaml

@dataclass
class Skill:
    name: str
    description: str
    body: str
    allowed_tools: Optional[str] = None
    license: Optional[str] = None
    compatibility: Optional[str] = None
    metadata: Optional[dict] = None

def parse_skill(skill_path: Path) -> Optional[Skill]:
    """Parse a SKILL.md file and return a Skill object."""
    if not skill_path.exists():
        return None

    content = skill_path.read_text()

    # Extract YAML frontmatter
    frontmatter_match = re.match(r'^---\s*\n(.*?)\n---\s*\n', content, re.DOTALL)
    if not frontmatter_match:
        return None

    try:
        frontmatter = yaml.safe_load(frontmatter_match.group(1))
    except yaml.YAMLError:
        return None

    # Required fields
    name = frontmatter.get("name")
    description = frontmatter.get("description")

    if not name or not description:
        return None

    # Validate name format (per spec)
    if not re.match(r'^[a-z0-9]+(-[a-z0-9]+)*$', name):
        return None

    if len(name) > 64 or len(description) > 1024:
        return None

    # Body is everything after frontmatter
    body = content[frontmatter_match.end():]

    return Skill(
        name=name,
        description=description,
        body=body.strip(),
        allowed_tools=frontmatter.get("allowed-tools"),
        license=frontmatter.get("license"),
        compatibility=frontmatter.get("compatibility"),
        metadata=frontmatter.get("metadata"),
    )

def list_skills(skills_dir: Path) -> List[Skill]:
    """List all valid skills in the skills directory."""
    skills = []

    if not skills_dir.exists():
        return skills

    for entry in skills_dir.iterdir():
        if entry.is_dir() and not entry.name.startswith("."):
            skill_path = entry / "SKILL.md"
            skill = parse_skill(skill_path)
            if skill:
                skills.append(skill)

    return sorted(skills, key=lambda s: s.name)
