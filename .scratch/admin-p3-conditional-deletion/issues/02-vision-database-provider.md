# Vision Database Provider selection seam

Type: task
Status: needs-info

## Decision đã chốt

Vision là Database Provider Instance và Template Provider Binding sẽ nhận `vision`.

## Information còn thiếu

`POST /mcp/vision/explain` hiện chỉ có `Device-Id` như metadata và chọn một process-wide
`effective_agent.providers.vision`. Cần chốt một trong các semantic selection sau trước khi
chuyển Vision vào DB desired state:

1. Resolve Device → Agent → effective Template và dùng Vision binding của snapshot đó cho mỗi
   request; hoặc
2. Giữ endpoint là deployment-wide service và không để Template binding chi phối endpoint.

Hai lựa chọn có ownership, cache/runtime provenance và behavior khi desired revision đổi khác
nhau. Không được chọn ngầm rồi materialize provider trong request.
