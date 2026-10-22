// Settings > Pet (wireframe W3): the pet library, name, colours, size, where the pet
// sits, tone, motion, the keyboard panel shortcut, and "Bring pet above clock". Every
// edit goes to the shell, which validates it, saves it in the local store and answers
// with the new view; the pet window follows immediately.
import { useEffect, useRef, useState } from 'preact/hooks';
import { insertArt, setPalette } from '../shared/art';
import { call, on, report } from '../shared/ipc';
import type { ColourVariant, Edge, PackCard, PetEdit, SettingsView, Tone } from '../shared/types';

const NAME_MAX = 20;

function Art({ svg, colours, size }: { svg: string; colours?: ColourVariant | undefined; size: number }) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (ref.current) insertArt(ref.current, svg);
  }, [svg]);
  useEffect(() => {
    if (ref.current && colours) setPalette(ref.current, colours.a, colours.b);
  }, [colours?.a, colours?.b]);
  useEffect(() => {
    if (ref.current) {
      ref.current.style.width = `${size}px`;
      ref.current.style.height = `${size}px`;
    }
  }, [size]);
  return <div class="art" ref={ref} />;
}

function PetCard({
  pack,
  chosen,
  onChoose,
}: {
  pack: PackCard;
  chosen: boolean;
  onChoose: () => void;
}) {
  const pending = pack.art_status === 'pending_review';
  return (
    <button
      type="button"
      role="radio"
      aria-checked={chosen}
      aria-disabled={!pack.selectable}
      class={`card${chosen ? ' chosen' : ''}${pending ? ' pending' : ''}`}
      title={pack.review}
      onClick={() => {
        if (pack.selectable) onChoose();
      }}
    >
      <Art svg={pack.art_svg} colours={pack.palette[0]} size={64} />
      <span class="card-name">{pack.name}</span>
      {pending ? <span class="pending-label">Art pending review</span> : null}
    </button>
  );
}

export function PetSettings() {
  const [view, setView] = useState<SettingsView | null>(null);
  const [name, setName] = useState('');
  const [size, setSize] = useState(100);
  const [status, setStatus] = useState('');

  const adopt = (v: SettingsView) => {
    setView(v);
    setName(v.name);
    setSize(v.size_pct);
  };

  useEffect(() => {
    let off: (() => void) | undefined;
    on('settings://changed', adopt)
      .then((u) => {
        off = u;
      })
      .catch(report);
    call('settings_bootstrap').then(adopt).catch(report);
    return () => off?.();
  }, []);

  if (!view) return <main class="pane" aria-busy="true" />;

  const edit = (e: PetEdit, done?: string) => {
    call('pet_edit', { edit: e })
      .then((v) => {
        adopt(v);
        setStatus(done ?? 'Saved.');
      })
      .catch((err: unknown) => {
        setStatus(String(err));
        if (view) adopt(view);
      });
  };

  const pack = view.packs.find((p) => p.id === view.pet_id);
  const colours = pack?.palette[view.colour_variant] ?? pack?.palette[0];
  const preset = view.presets.find((p) => p.pct === size);
  const previewPx = Math.round((128 * size) / 100);

  const saveName = () => {
    const n = name.trim();
    if (n === view.name) return;
    if (n.length === 0) {
      setStatus('Give your pet a name.');
      setName(view.name);
      return;
    }
    edit({ kind: 'rename', name: n });
  };

  return (
    <main class="pane">
      <h2>Pet</h2>

      <section aria-labelledby="lib-h">
        <h3 id="lib-h">Choose your pet</h3>
        <div class="grid" role="radiogroup" aria-labelledby="lib-h">
          {view.packs.map((p) => (
            <PetCard
              key={p.id}
              pack={p}
              chosen={p.id === view.pet_id}
              onChoose={() => edit({ kind: 'choose_pet', pet_id: p.id }, `${p.name} chosen.`)}
            />
          ))}
        </div>
        <p class="note">
          Pets marked "Art pending review" cannot be chosen yet; their art is still being drawn
          and reviewed.
        </p>
      </section>

      <div class="cols">
        <section aria-labelledby="look-h">
          <h3 id="look-h">Name and look</h3>
          <label class="field">
            <span>Name</span>
            <input
              type="text"
              value={name}
              maxLength={NAME_MAX}
              onInput={(e) => setName(e.currentTarget.value)}
              onBlur={saveName}
              onKeyDown={(e) => {
                if (e.key === 'Enter') saveName();
              }}
            />
          </label>

          {pack && pack.palette.length > 1 ? (
            <fieldset class="field">
              <legend>Colours</legend>
              <div class="swatches">
                {pack.palette.map((c, i) => (
                  <label class="swatch" key={c.name}>
                    <input
                      type="radio"
                      name="colour"
                      checked={i === view.colour_variant}
                      onChange={() => edit({ kind: 'colour', variant: i })}
                    />
                    <SwatchDot colours={c} />
                    <span>{c.name}</span>
                  </label>
                ))}
              </div>
            </fieldset>
          ) : null}

          <fieldset class="field">
            <legend>Size</legend>
            <div class="seg" role="group" aria-label="Size presets">
              {view.presets.map((p) => (
                <button
                  type="button"
                  class="seg-btn"
                  key={p.pct}
                  aria-pressed={size === p.pct}
                  title={`${p.label} (${p.pct}%)`}
                  onClick={() => {
                    setSize(p.pct);
                    edit({ kind: 'size', pct: p.pct });
                  }}
                >
                  {p.short}
                </button>
              ))}
            </div>
            <div class="slider">
              <input
                type="range"
                min={view.size_min}
                max={view.size_max}
                step={view.size_step}
                value={size}
                aria-label="Pet size"
                aria-valuetext={`${size}%${preset ? `, ${preset.label}` : ''}`}
                onInput={(e) => setSize(Number(e.currentTarget.value))}
                onChange={(e) => edit({ kind: 'size', pct: Number(e.currentTarget.value) })}
              />
              <output>{size}%</output>
            </div>
            <p class="note">
              {view.size_min}% to {view.size_max}%. The pet always stays inside the screen's work
              area at every size.
            </p>
          </fieldset>

          {pack ? (
            <div class="preview" aria-label="Preview on light and dark backgrounds">
              <div class="preview-light">
                <Art svg={pack.art_svg} colours={colours} size={previewPx} />
              </div>
              <div class="preview-dark">
                <Art svg={pack.art_svg} colours={colours} size={previewPx} />
              </div>
            </div>
          ) : null}
        </section>

        <section aria-labelledby="place-h">
          <h3 id="place-h">Where it sits</h3>
          <button
            type="button"
            class="btn primary"
            onClick={() => {
              call('bring_pet')
                .then(() => setStatus('Your pet is back above the clock.'))
                .catch(report);
            }}
          >
            Bring pet above clock
          </button>
          <p class="note">Also in the tray menu and the pet's own menu.</p>

          <fieldset class="field">
            <legend>Edge</legend>
            {(
              [
                ['right', 'Right edge'],
                ['left', 'Left edge'],
                ['free', 'Where I drop it'],
              ] as [Edge, string][]
            ).map(([value, label]) => (
              <label class="radio" key={value}>
                <input
                  type="radio"
                  name="edge"
                  checked={view.edge === value}
                  onChange={() => edit({ kind: 'edge', edge: value })}
                />
                {label}
              </label>
            ))}
          </fieldset>

          <label class="field">
            <span>Tone</span>
            <select
              value={view.tone}
              onChange={(e) => edit({ kind: 'tone', tone: e.currentTarget.value as Tone })}
            >
              <option value="gentle">Gentle</option>
              <option value="plain">Plain</option>
              <option value="cheerful">Cheerful</option>
            </select>
          </label>

          <label class="check">
            <input
              type="checkbox"
              role="switch"
              checked={view.motion}
              onChange={(e) => edit({ kind: 'motion', on: e.currentTarget.checked })}
            />
            Animate the pet
          </label>
          <p class="note">Windows' "Show animations" setting is respected as well.</p>

          <label class="field">
            <span>Keyboard pet panel</span>
            <select
              value={view.hotkey ?? ''}
              onChange={(e) => {
                const v = e.currentTarget.value;
                call('set_hotkey', { hotkey: v === '' ? null : v })
                  .then((nv) => {
                    adopt(nv);
                    setStatus(
                      nv.hotkey === null
                        ? 'Keyboard shortcut off.'
                        : nv.hotkey_registered
                          ? `${nv.hotkey} opens the pet panel.`
                          : `Windows did not accept ${nv.hotkey}. Choose another.`,
                    );
                  })
                  .catch((err: unknown) => setStatus(String(err)));
              }}
            >
              {view.hotkey_choices.map((h) => (
                <option value={h} key={h}>
                  {h}
                </option>
              ))}
              <option value="">Off</option>
            </select>
          </label>
          <p class="note">
            {view.hotkey === null
              ? 'Off. The panel still opens from the tray menu.'
              : view.hotkey_registered
                ? 'The panel takes focus; Escape closes it and returns you to your work.'
                : 'Windows did not accept this shortcut (another app may use it). Choose another.'}
          </p>

          <button
            type="button"
            class="btn"
            onClick={() => edit({ kind: 'reset_defaults' }, 'Pet settings reset.')}
          >
            Reset pet settings
          </button>
        </section>
      </div>

      <p class="status" role="status">
        {status}
      </p>
    </main>
  );
}

function SwatchDot({ colours }: { colours: ColourVariant }) {
  const ref = useRef<HTMLSpanElement>(null);
  useEffect(() => {
    if (ref.current) setPalette(ref.current, colours.a, colours.b);
  }, [colours.a, colours.b]);
  return <span class="dot" ref={ref} aria-hidden="true" />;
}
