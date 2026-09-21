# ADR 0001 — Một binary Rust duy nhất

## Status
Accepted

## Context
Server cá nhân không cần manager service, Redis, MySQL hay microservice orchestration.

## Decision
HTTP OTA, WebSocket voice transport và orchestration chạy trong một process Tokio/Axum.

## Consequences

Tốt: deploy đơn giản, ít failure mode, shared config rõ ràng.

Đổi lại: scale độc lập từng phần không phải mục tiêu V1.
