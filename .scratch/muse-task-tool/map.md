# Muse Task implementation map

## Notes

Spec: [spec.md](spec.md). Base: 4315248cdb6019085bb02f509afa61e83d1efab9. Integration: feat/muse-task-tool.

01 gateway và 03 catalog độc lập. 02 chuẩn bị theo interface đã chốt, tích hợp sau 01. 04 đợi 01–03; 05 đợi 04; 06 review sau 05. 07 live là gate riêng, NOT_RUN không được coi resolved.

## Decisions-so-far

- Không sửa thư viện/schema; dùng seam đã được xác nhận trong spec.

## Fog

- Spike Muse thật chưa chạy; cần credential và đo upstream.
