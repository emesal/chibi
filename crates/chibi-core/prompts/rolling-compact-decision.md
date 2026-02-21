You are deciding which conversation messages to archive during context compaction.

CURRENT MESSAGES (oldest first):
{MESSAGES}

{GOALS}{TODOS}EXISTING SUMMARY:
{SUMMARY}

Your task: Select approximately {TARGET_COUNT} messages to archive (move to summary).
Consider:
1. Keep messages directly relevant to current goals and todos
2. Keep recent messages (they provide immediate context)
3. Archive older messages that have been superseded or are less relevant
4. Preserve messages containing important decisions or key information
5. Tool call messages and their results should be archived together

Return ONLY a JSON array of message IDs to archive, e.g.: ["id1", "id2", "id3"]
No explanation, just the JSON array.
