# Muse Task implementation map

## Notes

Spec: [spec.md](spec.md). Base: 4315248cdb6019085bb02f509afa61e83d1efab9. Integration: feat/muse-task-tool.

01 gateway và 03 catalog độc lập. 02 chuẩn bị theo interface đã chốt, tích hợp sau 01. 04 xây theo port 373e5e99 và journal contract đã chốt, nghiệm thu đợi tích hợp 01/02; 05 đợi 04; 06 review sau 05. 07 live là gate riêng, NOT_RUN không được coi resolved.

## Decisions-so-far

- [03 catalog](issues/03-catalog.md) resolved; tích hợp ở 5d194db5. Mặc định OFF, tag Muse-only, bảo toàn prompt tùy chỉnh; HTTP/settings/Web/typecheck PASS.

- Không sửa thư viện/schema; dùng seam đã được xác nhận trong spec.

- [03 Catalog/Admin](issues/03-catalog.md): default OFF và live Muse provider selection; refresh discovery giữ Admin prompts; Admin HTTP, prompt upgrade, web UI/typecheck đã PASS.

## Fog

- Spike Muse thật chưa chạy; cần credential và đo upstream.
