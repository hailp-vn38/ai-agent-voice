# Obsidian / Knowledge Graph V3 — acceptance smoke

Status: **Manual test procedure; not executed inside CI.** Keep Obsidian's file editing disabled against a live canonical Knowledge root, or open a copy of the Vault. This is a verification gate from [ADR 0124](../adr/0124-wiki-wikilinks-graph-projection.md), not an external-write sync feature.

## Setup

1. Start the Rust server and Admin Web with the V3 Knowledge index successfully initialized.
2. In Knowledge Web, create physical Wiki folders AI/ and Rust/, then create AI/LLM.md, AI/Agent.md and Rust/Async.md using the existing Draft/Source verification and Publish flow.
3. Put these references in the canonical body of AI/Agent.md via the Wiki Link picker and save/publish:
   - [[AI/LLM]]
   - [[Rust/Async|Asynchronous Rust]]
   - [[AI/LLM#Inference]] (create an Inference heading inside LLM)
   - ![[photo.png]] and ![[audio.mp3]] (media **must not** create Wiki nodes/edges)
4. After publish, check Knowledge Web Graph (Global and Local 1/2 hops): Agent -> LLM and Agent -> Async are visible. Backlinks for LLM include Agent; edge inspector retains source, physical target, source line, alias and heading metadata. Media embeds are not listed as unresolved Wiki notes.
5. Copy the physical knowledge/wiki/ folder into a temporary directory. Open **that copied directory** as an Obsidian Vault. Do not open the broader knowledge/ root or write back to a live server Vault. Obsidian may create .obsidian/; it must not affect a subsequent server scan of the live Vault if one is used for read-only opening.
6. Open native Obsidian Graph and Backlinks. Confirm the same explicit Agent/LLM and Agent/Async relationships are recognized without importing the SQLite index or starting the Rust Graph API. Confirm display alias and heading navigation.
7. In the Web UI, attempt to rename AI/LLM.md while Agent still references [[AI/LLM]]; expect HTTP **409** with source note and link line, and no path mutation. First update/publish the backlink through normal CAS before retrying.
8. Verify no Raw, staged Drafts, Memory or revision-history notes appear in the Obsidian Vault. A staged edit of a verified Wiki must not modify Web Graph until publish.

## Regression commands

For a working repository checkout with Rust and Node installed:

    scripts/agent-test.sh targeted wiki_links
    scripts/agent-test.sh targeted wiki_graph
    scripts/agent-test.sh targeted local_graph
    scripts/agent-test.sh web test
    scripts/agent-test.sh web typecheck
    scripts/agent-test.sh web build
    scripts/agent-test.sh full

Record repository branch/commit, server configuration, the generated Vault copy, screenshots of Web Global/Local Graph and Obsidian native Graph/Backlinks, and whether all numbered checks passed. Do **not** claim Obsidian interoperability is certified based only on automated Markdown parsing tests.
