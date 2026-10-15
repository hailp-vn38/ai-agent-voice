"""Private local backup; reports contain counts/digests, never row contents."""
import hashlib
import json
import pathlib
import sqlite3
import sys
import tempfile

root = pathlib.Path(__file__).resolve().parent
state_path = root / "database-copy-state.json"

def snapshot(conn):
    tables = conn.execute("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name != '_sqlx_migrations' ORDER BY name").fetchall()
    result = {}
    for (table,) in tables:
        quoted = '"' + table.replace('"', '""') + '"'
        rows = sorted(map(repr, conn.execute(f"SELECT * FROM {quoted}").fetchall()))
        result[table] = {"rows": len(rows), "sha256": hashlib.sha256(repr(rows).encode()).hexdigest()}
    return result

if sys.argv[1] == "backup":
    path = pathlib.Path(tempfile.mkdtemp(prefix="plugin-migration-")) / "voice-agent.db"
    with sqlite3.connect("file:data/voice-agent.db?mode=ro", uri=True) as source, sqlite3.connect(path) as copy:
        source.backup(copy)
        before = snapshot(copy)
        version = copy.execute("SELECT max(version) FROM _sqlx_migrations").fetchone()[0]
        assert before["jobs"]["rows"] > 0
    state_path.write_text(json.dumps({"path": str(path), "from_version": version, "before": before}, indent=2) + "\n")
    print(f"Consistent backup ready: schema {version}, {before['jobs']['rows']} Jobs")
else:
    assert sys.argv[1] == "verify"
    state = json.loads(state_path.read_text())
    with sqlite3.connect(state["path"]) as copy:
        after = snapshot(copy)
        for table, expected in state["before"].items():
            assert after[table] == expected, f"Existing data changed: {table}"
        version = copy.execute("SELECT max(version) FROM _sqlx_migrations").fetchone()[0]
        assert version == 121
        assert copy.execute("PRAGMA foreign_key_check").fetchall() == []
        assert copy.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
    report = {"result": "PASS", "from_version": state["from_version"], "to_version": version, "existing_tables_preserved": len(state["before"]), "tables": state["before"], "foreign_keys": "PASS", "integrity": "PASS"}
    (root / "database-migration-report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(f"PASS: schema {state['from_version']} -> {version}; {len(state['before'])} existing tables unchanged, including {after['jobs']['rows']} Jobs")
