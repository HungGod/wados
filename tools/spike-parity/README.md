# The spike's answers, for wad-model's parity test

`crates/wad-model/tests/data/` holds what the v2 spike itself computes from its populated sample:
- `populated.json`: the spike's world, flattened to one look per wadspace;
- `parity.json`: every live wadspace's chips, suggestions and sign-in folders, and two Alt+Space searches.

`cargo test -p wad-model --test parity` checks that the Rust port gives the same answers. To make the files
again after the spike changes (from the wados root, with the spike beside it):

```bash
spike=../wadspaces-v2-spike out=$(mktemp -d)
cp tools/spike-parity/dump.test.ts "$spike/src/parity.dump.test.ts"
(cd "$spike" && OUT=$out npx vitest run src/parity.dump.test.ts); rm "$spike/src/parity.dump.test.ts"
python3 tools/spike-parity/flatten.py "$out/populated.json" crates/wad-model/tests/data/populated.json /dev/null
python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); print("{\n" + ",\n".join("  " + json.dumps(k) + ": " + json.dumps(v, ensure_ascii=False, separators=(",", ":")) for k, v in d.items()) + "\n}")' "$out/facts.json" > crates/wad-model/tests/data/parity.json
```
