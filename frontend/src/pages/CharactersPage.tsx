import { useEffect, useMemo, useState } from 'react';
import { api } from '../api/client';
import type { Attribute, BuildResult, Character, CharacterSummary, Content, FeatType, Skill } from '../api/types';
import { ATTRIBUTES, SKILLS } from '../api/types';
import { StatblockView } from '../components/StatblockView';
import { signed } from '../lib/format';

const blank = (c: Content): Character => ({
  id: '',
  name: 'New Hero',
  level: 1,
  ancestry_id: c.ancestries[0]?.id ?? '',
  background_id: c.backgrounds[0]?.id ?? '',
  class_id: c.classes[0]?.id ?? '',
  boosts: { ancestry_free: [], background: [], class_key: c.classes[0]?.key_attributes[0] ?? 'strength', level1_free: [] },
  trained_skills: [],
  feats: [],
  equipment: { weapons: [], armor: null, shield_id: null },
  spells: { cantrips: [], spells: [] },
});

function download(name: string, data: unknown) {
  const url = URL.createObjectURL(new Blob([JSON.stringify(data, null, 2)], { type: 'application/json' }));
  const a = document.createElement('a');
  a.href = url;
  a.download = name;
  a.click();
  URL.revokeObjectURL(url);
}

export function CharactersPage({ content }: { content: Content }) {
  const [list, setList] = useState<CharacterSummary[]>([]);
  const [ch, setCh] = useState<Character | null>(null);
  const [build, setBuild] = useState<BuildResult | null>(null);
  const [raw, setRaw] = useState(false);
  const [rawText, setRawText] = useState('');
  const [msg, setMsg] = useState<string | null>(null);

  const refresh = () => api.listCharacters().then(setList);
  useEffect(() => { refresh(); }, []);

  // Live validation (debounced).
  useEffect(() => {
    if (!ch) return;
    const t = setTimeout(() => api.validateCharacter(ch).then(setBuild).catch((e) => setMsg(e.message)), 250);
    return () => clearTimeout(t);
  }, [ch]);

  const cls = content.classes.find((c) => c.id === ch?.class_id);
  const anc = content.ancestries.find((a) => a.id === ch?.ancestry_id);
  const bg = content.backgrounds.find((b) => b.id === ch?.background_id);
  const freeSlots = ch?.boosts.ancestry_alternate ? 2 : (anc?.boosts.filter((b) => b === 'free').length ?? 0);
  const update = (f: (c: Character) => void) => setCh((c) => { if (!c) return c; const n = structuredClone(c); f(n); return n; });
  const errors = build?.report.issues.filter((i) => i.severity === 'error') ?? [];
  const warnings = build?.report.issues.filter((i) => i.severity === 'warning') ?? [];

  const featsBySlot = useMemo(() => {
    const kinds: FeatType[] = ['ancestry', 'class', 'skill', 'general', 'archetype'];
    return Object.fromEntries(kinds.map((k) => [k, content.feats.filter((f) => f.feat_type === k || (k === 'general' && f.feat_type === 'skill'))]));
  }, [content]);

  async function save() {
    if (!ch) return;
    try {
      const r = ch.id ? await api.saveCharacter(ch) : await api.createCharacter(ch);
      setCh(r.character);
      setMsg(r.build.report.issues.some((i) => i.severity === 'error') ? 'Saved (build has errors — it cannot enter combat until fixed)' : 'Saved — legal build');
      refresh();
    } catch (e) {
      setMsg((e as Error).message);
    }
  }

  async function importFile(file: File) {
    try {
      const r = await api.importCharacter(JSON.parse(await file.text()));
      setCh(r.character);
      setMsg(`Imported ${r.character.name}`);
      refresh();
    } catch (e) {
      setMsg(`Import failed: ${(e as Error).message}`);
    }
  }

  const attrSelect = (value: Attribute | undefined, onChange: (a: Attribute) => void, options: Attribute[] = ATTRIBUTES) => (
    <select value={value ?? ''} onChange={(e) => onChange(e.target.value as Attribute)}>
      <option value="" disabled>—</option>
      {options.map((a) => <option key={a} value={a}>{a}</option>)}
    </select>
  );

  return (
    <div className="page split">
      <aside>
        <div className="row">
          <button className="primary" onClick={() => { setCh(blank(content)); setBuild(null); }}>New</button>
          <label className="button">Import JSON<input type="file" accept="application/json" hidden onChange={(e) => e.target.files?.[0] && importFile(e.target.files[0])} /></label>
        </div>
        <ul className="list">
          {list.map((c) => (
            <li key={c.id} className={ch?.id === c.id ? 'on' : ''} onClick={() => api.getCharacter(c.id).then(setCh)}>
              <strong>{c.name}</strong> <span className="muted">L{c.level} {c.ancestry_id} {c.class_id}</span> {!c.is_legal && <span className="tag bad">illegal</span>}
            </li>
          ))}
        </ul>
      </aside>

      {ch && (
        <section className="editor">
          <div className="row">
            <input className="title" value={ch.name} onChange={(e) => update((c) => { c.name = e.target.value; })} />
            <label>Level <input type="number" className="narrow" min={1} max={20} value={ch.level} onChange={(e) => update((c) => { c.level = Number(e.target.value); })} /></label>
            <div className="spacer" />
            <button onClick={() => { setRaw(!raw); setRawText(JSON.stringify(ch, null, 2)); }}>{raw ? 'Form' : 'Raw JSON'}</button>
            <button onClick={() => download(`${ch.name || 'character'}.json`, ch)}>Export</button>
            {ch.id && <button onClick={async () => { await api.deleteCharacter(ch.id); setCh(null); refresh(); }}>Delete</button>}
            <button className="primary" onClick={save}>Save</button>
          </div>
          {msg && <div className="note" onClick={() => setMsg(null)}>{msg}</div>}

          {raw ? (
            <div>
              <textarea className="code" rows={30} value={rawText} onChange={(e) => setRawText(e.target.value)} />
              <button onClick={() => { try { setCh(JSON.parse(rawText)); setRaw(false); } catch (e) { setMsg((e as Error).message); } }}>Apply JSON</button>
            </div>
          ) : (
            <div className="form-grid">
              <fieldset>
                <legend>Origins</legend>
                <label>Ancestry
                  <select value={ch.ancestry_id} onChange={(e) => update((c) => { c.ancestry_id = e.target.value; c.boosts.ancestry_free = []; })}>
                    {content.ancestries.map((a) => <option key={a.id} value={a.id}>{a.name} ({a.hp} HP, {a.speed} ft)</option>)}
                  </select>
                </label>
                <label>Background
                  <select value={ch.background_id} onChange={(e) => update((c) => { c.background_id = e.target.value; c.boosts.background = []; })}>
                    {content.backgrounds.map((b) => <option key={b.id} value={b.id}>{b.name}</option>)}
                  </select>
                </label>
                <label>Class
                  <select value={ch.class_id} onChange={(e) => update((c) => { c.class_id = e.target.value; c.boosts.class_key = content.classes.find((x) => x.id === e.target.value)?.key_attributes[0] ?? 'strength'; })}>
                    {content.classes.map((x) => <option key={x.id} value={x.id}>{x.name}</option>)}
                  </select>
                </label>
              </fieldset>

              <fieldset>
                <legend>Attribute boosts</legend>
                <label className="inline"><input type="checkbox" checked={!!ch.boosts.ancestry_alternate} onChange={(e) => update((c) => { c.boosts.ancestry_alternate = e.target.checked; c.boosts.ancestry_free = []; })} /> Two free ancestry boosts (no fixed boosts/flaws)</label>
                <div className="row wrap">Ancestry free: {Array.from({ length: freeSlots }, (_, i) => <span key={i}>{attrSelect(ch.boosts.ancestry_free[i], (a) => update((c) => { c.boosts.ancestry_free[i] = a; }))}</span>)}</div>
                <div className="row wrap">Background: {attrSelect(ch.boosts.background[0], (a) => update((c) => { c.boosts.background[0] = a; }), bg?.boost_options)} {attrSelect(ch.boosts.background[1], (a) => update((c) => { c.boosts.background[1] = a; }))}</div>
                <div className="row wrap">Class key: {attrSelect(ch.boosts.class_key, (a) => update((c) => { c.boosts.class_key = a; }), cls?.key_attributes)}</div>
                <div className="row wrap">Free (4): {[0, 1, 2, 3].map((i) => <span key={i}>{attrSelect(ch.boosts.level1_free[i], (a) => update((c) => { c.boosts.level1_free[i] = a; }))}</span>)}</div>
                {[5, 10, 15, 20].filter((l) => l <= ch.level).map((l) => (
                  <div key={l} className="row wrap">Level {l}: {[0, 1, 2, 3].map((i) => <span key={i}>{attrSelect(ch.boosts.level_ups?.[l]?.[i], (a) => update((c) => { c.boosts.level_ups ??= {}; c.boosts.level_ups[l] ??= []; c.boosts.level_ups[l][i] = a; }))}</span>)}</div>
                ))}
                {build && <div className="attrs">{ATTRIBUTES.map((a) => <span key={a}><b>{a.slice(0, 3)}</b> {signed(build.attributes[a])}</span>)}</div>}
              </fieldset>

              <fieldset>
                <legend>Trained skills ({cls ? `${cls.additional_skills} + Int` : ''})</legend>
                <div className="checks">
                  {SKILLS.map((s) => (
                    <label key={s} className="inline">
                      <input type="checkbox" checked={ch.trained_skills.includes(s)} onChange={(e) => update((c) => { c.trained_skills = e.target.checked ? [...c.trained_skills, s] : c.trained_skills.filter((x) => x !== s); })} />
                      {s}{bg?.trained_skills.includes(s as Skill) ? ' (bg)' : ''}
                    </label>
                  ))}
                </div>
              </fieldset>

              <fieldset>
                <legend>Feats</legend>
                {ch.feats.map((f, i) => (
                  <div key={i} className="row">
                    <select value={f.slot_type} onChange={(e) => update((c) => { c.feats[i].slot_type = e.target.value as FeatType; })}>
                      {(['ancestry', 'class', 'skill', 'general', 'archetype'] as FeatType[]).map((k) => <option key={k}>{k}</option>)}
                    </select>
                    <input type="number" className="narrow" value={f.slot_level} min={1} max={20} onChange={(e) => update((c) => { c.feats[i].slot_level = Number(e.target.value); })} />
                    <select value={f.feat_id} onChange={(e) => update((c) => { c.feats[i].feat_id = e.target.value; })}>
                      <option value="">—</option>
                      {featsBySlot[f.slot_type]?.map((x) => <option key={x.id} value={x.id}>{x.name} ({x.level})</option>)}
                    </select>
                    <button onClick={() => update((c) => { c.feats.splice(i, 1); })}>✕</button>
                  </div>
                ))}
                <button onClick={() => update((c) => { c.feats.push({ feat_id: '', slot_type: 'class', slot_level: 1 }); })}>+ Feat</button>
              </fieldset>

              <fieldset>
                <legend>Equipment</legend>
                {ch.equipment.weapons.map((w, i) => (
                  <div key={i} className="row">
                    <select value={w.weapon_id} onChange={(e) => update((c) => { c.equipment.weapons[i].weapon_id = e.target.value; })}>
                      {content.weapons.map((x) => <option key={x.id} value={x.id}>{x.name}</option>)}
                    </select>
                    <label>+<input type="number" className="narrow" min={0} max={3} value={w.potency ?? 0} onChange={(e) => update((c) => { c.equipment.weapons[i].potency = Number(e.target.value); })} /></label>
                    <label>striking<input type="number" className="narrow" min={0} max={3} value={w.striking ?? 0} onChange={(e) => update((c) => { c.equipment.weapons[i].striking = Number(e.target.value); })} /></label>
                    <button onClick={() => update((c) => { c.equipment.weapons.splice(i, 1); })}>✕</button>
                  </div>
                ))}
                <button onClick={() => update((c) => { c.equipment.weapons.push({ weapon_id: content.weapons[0].id }); })}>+ Weapon</button>
                <label>Armor
                  <select value={ch.equipment.armor?.armor_id ?? ''} onChange={(e) => update((c) => { c.equipment.armor = e.target.value ? { armor_id: e.target.value } : null; })}>
                    <option value="">none</option>
                    {content.armor.map((x) => <option key={x.id} value={x.id}>{x.name} (+{x.ac_bonus})</option>)}
                  </select>
                </label>
                <label>Shield
                  <select value={ch.equipment.shield_id ?? ''} onChange={(e) => update((c) => { c.equipment.shield_id = e.target.value || null; })}>
                    <option value="">none</option>
                    {content.shields.map((x) => <option key={x.id} value={x.id}>{x.name}</option>)}
                  </select>
                </label>
              </fieldset>

              {cls?.spellcasting && (
                <fieldset>
                  <legend>Spells ({cls.spellcasting.tradition})</legend>
                  <div className="checks">
                    {content.spells.filter((s) => s.traditions.includes(cls.spellcasting!.tradition)).map((s) => {
                      const on = s.cantrip ? ch.spells.cantrips.includes(s.id) : ch.spells.spells.some((k) => k.spell_id === s.id);
                      return (
                        <label key={s.id} className="inline" title={s.description}>
                          <input
                            type="checkbox"
                            checked={on}
                            onChange={(e) => update((c) => {
                              if (s.cantrip) c.spells.cantrips = e.target.checked ? [...c.spells.cantrips, s.id] : c.spells.cantrips.filter((x) => x !== s.id);
                              else c.spells.spells = e.target.checked ? [...c.spells.spells, { spell_id: s.id, rank: s.rank }] : c.spells.spells.filter((k) => k.spell_id !== s.id);
                            })}
                          />
                          {s.name} {s.cantrip ? '(cantrip)' : `(${s.rank})`}
                        </label>
                      );
                    })}
                  </div>
                </fieldset>
              )}
            </div>
          )}

          <div className="validation">
            {errors.length === 0 ? <div className="good">✓ Legal build</div> : errors.map((i, k) => <div key={k} className="bad">✕ {i.message} <span className="muted">({i.path})</span></div>)}
            {warnings.map((i, k) => <div key={k} className="muted">! {i.message}</div>)}
          </div>
          {build?.statblock && <StatblockView sb={build.statblock} />}
        </section>
      )}
    </div>
  );
}
