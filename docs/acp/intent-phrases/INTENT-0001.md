# INTENT-0001: LLM Similar-Sentence Intent Phrases

## Purpose
Central phrase list for routing user requests to the **LLM-only similar-sentence cut flow**.

## Runtime Binding
- ACP reads this file at runtime.
- If user prompt contains any phrase in the intent block below, ACP routes to:
  - `run_llm_similarity_only_flow`
- For this intent, the product meaning of "cut similar sentences" is:
  - **LLM analysis first**
  - then `validate_edit_plan`
  - then `apply_edit_plan`

## Authoring Rules
- Keep one phrase per bullet.
- Use imperative command style.
- Keep phrases short and practical.
- Add aliases freely across languages.

<!-- ACP_INTENT_PHRASES_START -->
- `cut similar sentences`
- `delete similar sentences`
- `remove similar sentences`
- `trim similar sentences`
- `cut duplicate subtitles`
- `delete duplicate subtitles`
- `remove duplicate subtitles`
- `cut repeated subtitles`
- `delete repeated subtitles`
- `llm cut similar sentences`
- `use llm cut similar sentences`
- `use llm to cut similar subtitles`

- `刪相似字句`
- `删除相似字句`
- `刪除相似字句`
- `删除相似句子`
- `刪重複字句`
- `刪除重複字幕`
- `刪重複字幕`
- `用LLM刪相似字句`
- `用 llm 刪相似字句`
- `用LLM刪重複字幕`

- `相似語句を削除`
- `重複字幕を削除`
- `似た字幕を削除`
- `LLMで重複字幕を削除`

- `유사 문장 삭제`
- `중복 자막 삭제`
- `비슷한 자막 삭제`
- `LLM으로 유사 문장 삭제`
<!-- ACP_INTENT_PHRASES_END -->

## Notes
- This file is for routing phrases only.
- It does not define edit-plan math or thresholds.
