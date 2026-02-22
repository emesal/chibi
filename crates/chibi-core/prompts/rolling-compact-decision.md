You are deciding which conversation messages to archive during context compaction.

CURRENT MESSAGES (oldest first):
{MESSAGES}

{GOALS}{TODOS}EXISTING SUMMARY:
{SUMMARY}

Your task: Select approximately {TARGET_COUNT} messages to archive (move to summary).

Use the goals and todos above as your primary relevance criterion — messages that
directly serve active goals or todos should be kept; messages that are superseded,
tangential, or no longer actionable are good candidates for archival.

Also consider:
- Keep recent messages (they provide immediate context)
- Archive older messages that have been resolved or are less relevant to current goals
- Preserve messages containing important decisions or key information not yet in the summary
- Tool call messages and their results should be archived together

Return ONLY a JSON array of message IDs to archive, e.g.: ["id1", "id2", "id3"]
No explanation, just the JSON array.
