"""Ingest forge-core NDJSON index output into LadybugDB code tables."""
import json
from forge_graph.db import GraphDB


def ingest_symbols(db: GraphDB, ndjson: str) -> int:
    """Parse NDJSON, MERGE nodes/edges into LadybugDB. Returns count."""
    count = 0

    for line in ndjson.strip().split("\n"):
        if not line.strip():
            continue
        sym = json.loads(line)
        kind = sym["kind"]

        if kind == "file":
            db.conn.execute(
                "MERGE (f:File {id: $id}) "
                "SET f.file_path = $fp, f.name = $name, f.language = $lang, "
                "f.size_bytes = $size",
                parameters={
                    "id": sym["id"], "fp": sym["file_path"],
                    "name": sym["name"], "lang": sym.get("language", "unknown"),
                    "size": sym.get("size_bytes", 0),
                },
            )

        elif kind == "function":
            db.conn.execute(
                "MERGE (fn:Function {id: $id}) "
                "SET fn.name = $name, fn.file_path = $fp, "
                "fn.line_start = $ls, fn.line_end = $le, fn.signature = $sig",
                parameters={
                    "id": sym["id"], "name": sym["name"], "fp": sym["file_path"],
                    "ls": sym.get("line_start", 0), "le": sym.get("line_end", 0),
                    "sig": sym.get("signature", ""),
                },
            )
            _create_contains(db, sym["file_path"], sym["id"], "Function")

        elif kind == "class":
            db.conn.execute(
                "MERGE (c:Class {id: $id}) "
                "SET c.name = $name, c.file_path = $fp, "
                "c.line_start = $ls, c.line_end = $le",
                parameters={
                    "id": sym["id"], "name": sym["name"], "fp": sym["file_path"],
                    "ls": sym.get("line_start", 0), "le": sym.get("line_end", 0),
                },
            )
            _create_contains(db, sym["file_path"], sym["id"], "Class")

        elif kind == "method":
            db.conn.execute(
                "MERGE (m:Method {id: $id}) "
                "SET m.name = $name, m.file_path = $fp, "
                "m.line_start = $ls, m.line_end = $le, "
                "m.signature = $sig, m.class_id = $cid",
                parameters={
                    "id": sym["id"], "name": sym["name"], "fp": sym["file_path"],
                    "ls": sym.get("line_start", 0), "le": sym.get("line_end", 0),
                    "sig": sym.get("signature", ""),
                    "cid": sym.get("class_id", ""),
                },
            )
            _create_contains(db, sym["file_path"], sym["id"], "Method")

        count += 1

    return count


def _create_contains(db: GraphDB, file_path: str, target_id: str, target_label: str) -> None:
    """Create CONTAINS edge from File -> target. Idempotent via MERGE.

    NOTE: target_label is an f-string interpolation for the Cypher label name.
    This is safe because the caller only passes hardcoded values
    ("Function", "Class", "Method"), never user input.
    """
    db.conn.execute(
        f"MATCH (f:File {{file_path: $fp}}), (t:{target_label} {{id: $tid}}) "
        "MERGE (f)-[:CONTAINS]->(t)",
        parameters={"fp": file_path, "tid": target_id},
    )
