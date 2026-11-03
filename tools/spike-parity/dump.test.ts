import { it } from 'vitest';
import { writeFileSync } from 'node:fs';
import { act, world } from './sim/testing';
import { PASSWORD } from './sim/scenarios';
import { chipsOf } from './sim/chips';
import { suggestions } from './sim/suggest';
import { foldersOf } from './sim/folders';
import { search } from './sim/variants';
import { liveWadspaces } from './sim/world';

it('dump', () => {
  const w = world('populated');
  act(w, 'power.unlock', PASSWORD);
  const out = { machine: w.machine.id, seq: w.seq, wadspaces: w.wadspaces, secrets: w.secrets, tags: w.tags, steam: w.steam, discs: w.discs, gamePackages: w.gamePackages, browser: w.machine.browser };
  writeFileSync(process.env.OUT + '/populated.json', JSON.stringify(out, null, 1));
  const facts: Record<string, unknown> = {};
  for (const x of liveWadspaces(w)) {
    facts[x.id] = { chips: chipsOf(x).map((c) => `${c.kind}:${c.id}${c.launch ? '*' : ''}`), sugg: suggestions(x).map((s) => `${s.item}${s.why ? ' (' + s.why + ')' : ''}`), folders: x.apps.map((a) => [a.app, foldersOf(x, a.app).map((f) => [f.home, f.key?.name ?? null, f.items.map((v) => v.id)])]) };
  }
  facts['search:'] = search(w, '').map((f) => `${f.ws.id}/${f.v.id}`);
  facts['search:git'] = search(w, 'git').map((f) => `${f.ws.id}/${f.v.id}`);
  writeFileSync(process.env.OUT + '/facts.json', JSON.stringify(facts, null, 1));
});
