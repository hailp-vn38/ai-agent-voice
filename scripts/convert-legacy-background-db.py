#!/usr/bin/env python3
"""Convert the legacy 105–108 MCP Background schema to executable Job schema.

Run with the server stopped. --dry-run validates a copy without changing the DB.
Legacy settings/installations remain archived; Background requires fresh activation.
"""
import argparse
import fcntl
import hashlib
import os
from pathlib import Path
import sqlite3
import tempfile


LEGACY = {
    105: "6f11fc6b1cb8d7c2c35ad92041f7d148b8a63b737006a8626d82c0efe9754287b534aef45be929539e81d48732f24ee6",
    106: "0c435ecf2439143d732598c02418bcf9cf174955267ebf6ed41019364eafed1d54ea3285708e79e66b5b881458f988af",
    107: "12aad24dfd9f9cdf0306313bf91d970c80689c2d379149e6047f22dfc0481d61f16fbac57bb928b7c79aca50aa1ae7f7",
    108: "6f1afcd4e2b61090b964049d8181a86cba5948ba4751c285d69f94e8ace7b7009df32d96c77fa8e027959af196092f5f",
}
MIGRATIONS = Path(__file__).resolve().parent.parent / "crates/voice-agent-server/migrations"


def rows(db, table):
    return db.execute(f'SELECT * FROM "{table}" ORDER BY rowid').fetchall()


def validate(db):
    applied = db.execute("SELECT version,checksum,success FROM _sqlx_migrations").fetchall()
    versions = {version: checksum.hex() for version, checksum, success in applied if success}
    if any(not success for _, _, success in applied):
        raise ValueError("Database has a failed migration")
    if {v: digest for v, digest in versions.items() if v >= 105} != LEGACY:
        raise ValueError("Not the supported legacy 105–108 schema; refusing conversion")
    for version, digest in versions.items():
        if version >= 105:
            continue
        matches = list(MIGRATIONS.glob(f"{version:04d}_*.sql"))
        if len(matches) != 1 or hashlib.sha384(matches[0].read_bytes()).hexdigest() != digest:
            raise ValueError(f"Base migration {version} differs; refusing conversion")
    names = {name for (name,) in db.execute("SELECT name FROM sqlite_master WHERE type='table'")}
    if names & {"jobs", "job_steps", "job_artifacts", "legacy_background_settings",
                "legacy_plugin_installations", "legacy_sqlx_migrations"}:
        raise ValueError("Unexpected Job/archive tables; refusing conversion")
    if db.execute("PRAGMA integrity_check").fetchall() != [("ok",)]:
        raise ValueError("Database integrity check failed")
    if db.execute("PRAGMA foreign_key_check").fetchall():
        raise ValueError("Database has broken foreign keys")


def statements(db, sql):
    # executescript() commits implicitly; keep the conversion in one transaction.
    statement = ""
    for line in sql.splitlines(keepends=True):
        statement += line
        if sqlite3.complete_statement(statement):
            db.execute(statement)
            statement = ""
    if statement.strip():
        raise ValueError("Incomplete migration SQL")


def convert(db):
    validate(db)
    tables = [name for (name,) in db.execute(
        "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'"
    )]
    unchanged = {name: rows(db, name) for name in tables
                 if name not in {"_sqlx_migrations", "background_settings", "plugin_installations"}}
    settings = rows(db, "background_settings")
    plugins = rows(db, "plugin_installations")
    db.execute("ALTER TABLE background_settings RENAME TO legacy_background_settings")
    db.execute("ALTER TABLE plugin_installations RENAME TO legacy_plugin_installations")
    db.execute("CREATE TABLE legacy_sqlx_migrations AS SELECT * FROM _sqlx_migrations WHERE version>=105")
    db.execute("DELETE FROM _sqlx_migrations WHERE version>=105")
    for version in (105, 106):
        path, = MIGRATIONS.glob(f"{version:04d}_*.sql")
        statements(db, path.read_text())
        db.execute(
            "INSERT INTO _sqlx_migrations(version,description,success,checksum,execution_time) VALUES(?,?,1,?,0)",
            (version, path.stem.split("_", 1)[1].replace("_", " "), hashlib.sha384(path.read_bytes()).digest()),
        )
    db.execute("""UPDATE background_settings SET
        orchestrator_llm_provider=(SELECT orchestrator_llm_provider FROM legacy_background_settings WHERE id=1),
        worker_llm_provider=(SELECT worker_llm_provider FROM legacy_background_settings WHERE id=1),
        revision=(SELECT revision+1 FROM legacy_background_settings WHERE id=1), enabled=0""")
    # Check preservation before commit; any mismatch rolls back the entire conversion.
    if (not all(rows(db, name) == before for name, before in unchanged.items())
            or rows(db, "legacy_background_settings") != settings
            or rows(db, "legacy_plugin_installations") != plugins
            or db.execute("SELECT COUNT(*) FROM jobs").fetchone() != (0,)
            or db.execute("PRAGMA foreign_key_check").fetchall()
            or db.execute("PRAGMA integrity_check").fetchall() != [("ok",)]):
        raise ValueError("Converted database failed preservation/integrity checks")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("database", type=Path)
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()
    path = args.database.resolve(strict=True)
    with open(str(path) + ".background.lock", "a+b") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        with sqlite3.connect(f"file:{path}?mode=rw", uri=True) as original:
            original.execute("PRAGMA foreign_keys=ON")
            original.execute("BEGIN IMMEDIATE")
            validate(original)
            if args.dry_run:
                with sqlite3.connect(":memory:") as copy:
                    with sqlite3.connect(f"file:{path}?mode=ro", uri=True) as reader:
                        reader.backup(copy)
                    copy.execute("PRAGMA foreign_keys=ON")
                    copy.execute("BEGIN IMMEDIATE")
                    convert(copy)
                    copy.commit()
                original.rollback()
                print("PASS: conversion and data-preservation checks on a copy")
            else:
                descriptor, backup_name = tempfile.mkstemp(prefix=path.name + ".pre-job-schema-", suffix=".bak", dir=path.parent)
                os.close(descriptor)
                with sqlite3.connect(backup_name) as backup:
                    with sqlite3.connect(f"file:{path}?mode=ro", uri=True) as reader:
                        reader.backup(backup)
                print(f"BACKUP: {backup_name}", flush=True)
                convert(original)
                original.commit()
                print("PASS: converted legacy schema; application rows preserved; Background disabled")


if __name__ == "__main__":
    main()
