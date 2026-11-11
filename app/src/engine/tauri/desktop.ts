// Desktop entry (inside the Tauri shell): the sounds list comes from the sound folders under
// PRAXIS_HOME/sounds, and every request goes through TauriEngine to the real engine; playback
// goes through the real-time engine (crates/praxis-app/src/audio.rs).

import { Shell, type SoundEntry } from '../../ui/shell';
import { tauriBridge, type Bridge } from './bridge';
import type { SoundInfo } from './protocol';
import { mountPlaybackLine } from './playbackLine';
import { TauriEngine } from './TauriEngine';

function entry(engine: TauriEngine, s: SoundInfo): SoundEntry {
  return {
    id: s.id,
    name: s.name,
    sub: s.sub,
    // A folder that is no longer a sound is refused: say why and start a new sound instead.
    open: () =>
      engine.openSound(s.path).catch((e: unknown) => {
        console.error(e instanceof Error ? e.message : e);
        return engine.openSound(null);
      }),
  };
}

/** Builds the workstation on the real engine and opens the newest sound (or a new one). */
export async function startDesktop(root: HTMLElement, bridge?: Bridge): Promise<{ shell: Shell; engine: TauriEngine }> {
  const engine = new TauriEngine(bridge ?? (await tauriBridge()));
  const list = await engine.listSounds();
  const sounds: SoundEntry[] = list.map((s) => entry(engine, s));
  sounds.push({ id: 'new', name: 'Untitled sound', sub: 'New', open: () => engine.openSound(null) });
  const shell = new Shell(root, engine, sounds);
  // Which engine plays, the device, xruns and the level meters, under the Sound panel's title.
  mountPlaybackLine(root, engine);
  // A new sound gets its folder with its first take: list it and switch to it (the shell keeps
  // the same array, so the new row shows on its next render).
  engine.onSoundCreated((s) => {
    if (!sounds.some((x) => x.id === s.id)) sounds.unshift(entry(engine, s));
    void shell.openSound(s.id);
  });
  const wanted = new URLSearchParams(location.search).get('sound');
  await shell.openSound(wanted && sounds.some((s) => s.id === wanted) ? wanted : (list[0]?.id ?? 'new'));
  return { shell, engine };
}
