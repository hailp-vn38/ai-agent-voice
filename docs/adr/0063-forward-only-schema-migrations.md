# ADR 0063 — Forward-only Schema Migrations

## Status

Accepted

SQLx migration history là authoritative và schema version monotonic forward-only. Startup kiểm tra DB version không mới hơn migration embedded của binary, rồi mới apply pending forward migrations. DB schema mới hơn binary trả `database_schema_incompatible`; migration failure cũng fail trước listener. `migrate_on_start=false` chỉ cho boot nếu schema đã current. Không reverse migration, drop schema để rollback, ignore unknown schema hoặc best-effort startup.

Backup/restore không thuộc application V1. Operator dùng SQLite-consistent backup API/CLI hoặc stop process trước copy (đặc biệt WAL), rồi deploy/verify release. Rollback release là restore compatible backup trước khi chạy binary cũ. Migrations transaction-safe khi SQLite operation cho phép; failure không cho application chạy với partial schema.
