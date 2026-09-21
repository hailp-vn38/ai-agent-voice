# Testing 03 — ASR module

## Mock HTTP contract

Mock server nên kiểm tra request audio metadata và trả response fixture.

Cases:

- success -> normalized text.
- Vietnamese Unicode giữ nguyên.
- upstream 401 -> `AsrError::Auth` hoặc mapped error.
- 500 -> provider error.
- invalid JSON -> provider protocol error.
- timeout -> timeout error.
- empty text -> actor không khởi tạo LLM turn.

## Generation test

1. start generation 10.
2. gửi ASR request chậm.
3. actor chuyển sang generation 11.
4. ASR generation 10 trả về.
5. assert không gửi STT và không gọi LLM.

## Command

```bash
./scripts/test-module.sh asr
```
