# Flatten the spike's populated world (dump/populated.json) into a wad-model Library fixture.
import json, sys
src, out_lib, out_build = sys.argv[1], sys.argv[2], sys.argv[3]
d = json.load(open(src))
live = [w for w in d['wadspaces'] if w.get('deletedAt') is None]
live.sort(key=lambda w: w['order'])
wss, build = [], {}
for i, w in enumerate(live):
    c = next(c for c in w['customizations'] if c['id'] == w['defaultCustomization'])
    v = next(v for v in c['variants'] if v['id'] == c['defaultVariant'])
    look = {k: v[k] for k in ('wallpaper', 'projects', 'autostart', 'layout')}
    for k in ('launch', 'icon', 'titleBars'):
        if k in v: look[k] = v[k]
    for p in look['projects']:
        if p.get('make') is False: del p['make']
    # An add-on without its app (the sample's Cosmic Bodybuilding has Prettier and no VS Code) isn't a state edits can make.
    apps = {a['app'] for a in w['apps']}
    w['exts'] = [e for e in w['exts'] if e['app'] in apps]
    x = {'version': 6, 'id': w['id'], 'machine': w['machine'], 'order': i, 'name': w['name'],
         'apps': w['apps'], 'exts': w['exts'], 'secrets': w['secrets'], 'look': look}
    if c.get('tag'): x['tag'] = c['tag']
    x['createdAt'] = w['createdAt']; x['updatedAt'] = w['updatedAt']
    for k in ('agents', 'lastLaunched', 'starter', 'category', 'commands'):
        if k in w: x[k] = w[k]
    wss.append(x)
    build[w['id']] = {'built': w['built'], 'sizeMb': w['sizeMb']}
keys = [{k: s[k] for k in ('id', 'name', 'kind', 'paths', 'size', 'createdAt', 'changedAt', 'usedBy', 'sites') if k in s} for s in d['secrets']]
lib = {'wadspaces': wss, 'tags': d['tags'], 'keys': keys, 'webApps': d['browser']['apps'], 'steam': d['steam'], 'discs': d['discs'], 'seq': 100}
def dump(o, f):
    # One wadspace (or key) per line: small, and diffs stay readable.
    with open(f, 'w') as fh:
        fh.write('{\n')
        items = list(o.items())
        for n, (k, v) in enumerate(items):
            end = ',' if n < len(items) - 1 else ''
            if isinstance(v, list) and v:
                fh.write(f'  "{k}": [\n')
                for m, e in enumerate(v):
                    fh.write('    ' + json.dumps(e, ensure_ascii=False, separators=(',', ':')) + (',' if m < len(v) - 1 else '') + '\n')
                fh.write(f'  ]{end}\n')
            else:
                fh.write(f'  "{k}": ' + json.dumps(v, ensure_ascii=False, separators=(',', ':')) + end + '\n')
        fh.write('}\n')
dump(lib, out_lib)
dump(build, out_build)
