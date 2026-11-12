"""Writes docs/cli/commands.md from the release `praxis` binary: every command's --help and a real
JSON result (trimmed: arrays to 2 items, strings to 160 characters, depth 4), so the docs cannot
drift from the code.

    cargo build --release -p praxis-cli
    python -I docs/cli/generate.py            # from the repository root

Uses the offline planner (no API key, no network), a temporary PRAXIS_HOME, and the felt piano
pack from PRAXIS_PACKS (falls back to ./packs). Plays no audio.
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile

ROOT = os.getcwd()
EXE = os.path.join(ROOT, 'target', 'release', 'praxis.exe' if os.name == 'nt' else 'praxis')
FX = os.path.join(ROOT, 'fixtures')
HOME = tempfile.mkdtemp(prefix='praxis-cli-docs-')
WORK = os.path.join(HOME, 'work')
os.makedirs(WORK)
ENV = dict(os.environ, PRAXIS_HOME=HOME, PRAXIS_KEYCHAIN='off')
ENV.setdefault('PRAXIS_PACKS', os.path.join(ROOT, 'packs'))
ENV.pop('ANTHROPIC_API_KEY', None)


def trim(v, depth=0, limit=4):
    if isinstance(v, dict):
        if depth >= limit:
            return {k: '...' for k in list(v)[:6]}
        return {k: trim(x, depth + 1, limit) for k, x in v.items()}
    if isinstance(v, list):
        t = [trim(x, depth + 1, limit) for x in v[:2]]
        if len(v) > 2:
            t.append(f'... {len(v) - 2} more')
        return t
    if isinstance(v, str):
        v = v.replace(HOME, '<PRAXIS_HOME>').replace(ROOT, '<repo>')
        return v if len(v) <= 160 else v[:157] + '...'
    if isinstance(v, float):
        return round(v, 3)
    return v


def help_text(*cmd):
    p = subprocess.run([EXE, *cmd, '--help'], env=ENV, capture_output=True, text=True, encoding='utf8')
    return p.stdout.replace('praxis.exe', 'praxis').strip()


def run(*args, stdin=None, depth=4):
    p = subprocess.run([EXE, '--json', *args], cwd=WORK, env=ENV, capture_output=True, text=True,
                       encoding='utf8', input=stdin)
    v = json.loads(p.stdout)
    shown = ' '.join(a.replace(WORK + os.sep, '').replace(FX + os.sep, 'fixtures/')
                     .replace(ROOT + os.sep, '') for a in args)
    return {'cmd': f'praxis --json {shown}', 'exit': p.returncode, 'json': trim(v, 0, depth)}


def w_path(name):
    return os.path.join(WORK, name)


# (title, help path, intro, [example runs]) in the order of `praxis --help`.
SECTIONS = []


def section(title, help_path, intro, *examples):
    SECTIONS.append((title, help_path, intro, list(examples)))


A1 = os.path.join(FX, 'acceptance1.score.json')
bad = json.load(open(A1, encoding='utf8'))
bad['tempo']['bpm'] = 0
json.dump(bad, open(w_path('bad.score.json'), 'w', encoding='utf8'))
BRIEF = 'a calm felt piano loop in A minor, 16 bars at 128 BPM, with a soft kick from bar 9'

section('render', ['render'], 'Renders a score or project to WAV or FLAC and checks the result against the score '
        '(`check` in the payload). `--stems DIR` writes one file per layer.',
        run('render', A1, '-o', w_path('a1.wav')))
section('validate', ['validate'], 'Validates a score or project. An invalid score is an `invalid_score` error with '
        'one issue per problem.',
        run('validate', A1), run('validate', w_path('bad.score.json')))
section('schema', ['schema'], 'Prints the JSON Schemas of the Score, the ScorePatch and the ScorePlan an agent can '
        'write (`--write` refreshes `schema/`).', run('schema', depth=2))
section('generate', ['generate'], 'Plans a new project from a brief (`--prompt`; offline rules, or Claude with a '
        'key) or composes a ScorePlan an agent wrote (`--plan`). `--render` also renders and checks it.',
        run('generate', '--provider', 'offline', '--prompt', BRIEF, '-o', w_path('calm.praxis'),
            '--render', w_path('calm.wav'), depth=3))
section('edit', ['edit'], 'A scoped edit of a project, from words or an EditPlan; only the bars and layers it names '
        'change. The project keeps an undo history.',
        run('edit', w_path('calm.praxis'), '--provider', 'offline', '--prompt',
            'change the last four bars to add a subtle melody'))
section('variations', ['variations'], 'N alternates of a selection, each rendered and checked; nothing is committed '
        '(keep one with `keep-variation`).', None)
section('ask', ['ask'], 'Makes a take in a sound folder: the first request is the brief, later ones are scoped edits '
        'of the current take (`--bars` with `--layer`), a hand edit (`--patch`), or a request about a reference '
        'clip (`--reference`). With an API key the sound assistant carries it out.',
        run('ask', 'calm', BRIEF, '--provider', 'offline', depth=3),
        run('ask', 'calm', 'make it softer', '--bars', '13-16', '--layer', 'piano', '--provider', 'offline'))
SECTIONS[-2] = ('variations', ['variations'], SECTIONS[-2][2],
                [run('variations', '--sound', 'calm', '--bars', '13-16', '--layer', 'piano', '-n', '2', depth=3)])
section('takes', ['takes'], 'The take history of a sound: every take with its request, change, checks and render.',
        run('takes', 'calm', depth=3))
section('restore', ['restore'], 'Goes back to take N as an undoable change.', None)
section('keep-variation', ['keep-variation'], 'Commits a variation as a new take.', run('keep-variation', 'calm', '1', depth=3))
SECTIONS[-2] = ('restore', ['restore'], SECTIONS[-2][2], [run('restore', 'calm', '1', depth=3)])
section('open-view', ['open-view'], 'Asks the app (if it has the sound open) to show a panel view.',
        run('open-view', 'calm', 'notes', '--who', 'cut-assistant'))
section('check', ['check'], 'Measures a render against its score: tempo, key, grid, loudness, true peak, clipping, '
        'loop seam, silence. Exits 1 if a check fails; "can\'t tell" checks do not count.',
        run('check', w_path('a1.wav'), '--score', A1))
section('analyze', ['analyze'], 'The complete Sound Profile of an audio or video clip (kind, tempo, key, loudness, '
        'spectrum, rhythm, envelope, pitch, features).', run('analyze', w_path('a1.wav')))
section('library', ['library'], 'Saved, tagged Sound Profiles (never audio) under `PRAXIS_HOME/library`.',
        run('library', 'add', w_path('a1.wav'), '--name', 'calm piano', '--tag', 'piano'),
        run('library', 'list'), run('library', 'tag', 'calm-piano', '--add', 'loop'),
        run('library', 'remove', 'calm-piano'))
section('match', ['match'], 'Makes a new project with a reference clip\'s character (never its melody), refined by '
        'rendering and measuring; reports the similarity per iteration and the originality check.',
        run('match', w_path('a1.wav'), '--prompt', 'calm', '--max-iterations', '10', '-o', w_path('like.praxis')))
section('export-midi', ['export-midi'], 'A Standard MIDI File of a score or project, one track per layer.',
        run('export-midi', w_path('calm.praxis'), '-o', w_path('calm.mid')))
section('packs', ['packs'], 'Instrument packs from `packs/index.json`: `list`, and `install NAME` (downloads over '
        'https, checks the exact size and blake3, unpacks through a staging folder).', run('packs', 'list'))
section('presets', ['presets'], 'Every instrument preset a ScorePlan may name, and whether it renders here.',
        run('presets'))
section('bench', ['bench'], 'Times a render on this machine: the built-in 2-minute, 8-layer piece (sampled felt '
        'piano, FM keys, bass, pluck, granular texture, kick, hat, rim, with their effects and the master bus) '
        'against the 20x real-time target. `realtime_factor` is from the median run; `master_hash` is equal '
        'when two builds render bit-identical audio. Writes nothing (`--save-piece` writes the score).',
        run('bench', '--seconds', '8', '--runs', '1'))
section('key', ['key'], 'Stores, clears or reports the Anthropic API key in the OS keychain. `set` reads the key from '
        'stdin only; it is never printed.', run('key', 'status'))

out = ['# `praxis` commands', '',
       'Generated by `docs/cli/generate.py` from the release build: each command\'s `--help` and a real',
       '`--json` result (trimmed: arrays show 2 items, long strings are cut, paths shortened). Field by',
       'field contract: [../contracts/cli.md](../contracts/cli.md). Overview, errors and environment:',
       '[README.md](README.md).', '', '```text', help_text(), '```', '']
for title, path, intro, examples in SECTIONS:
    out += [f'## `{title}`', '', intro, '', '```text', help_text(*path), '```', '']
    subs = {'library': ['add', 'list', 'show', 'tag', 'remove'], 'packs': ['list', 'install'],
            'key': ['set', 'clear', 'status']}.get(title, [])
    for s in subs:
        out += [f'`{title} {s}`:', '', '```text', help_text(title, s), '```', '']
    for ex in examples:
        out += [f'Example (exit {ex["exit"]}):', '', '```bash', ex['cmd'], '```', '', '```json',
                json.dumps(ex['json'], indent=2, ensure_ascii=False), '```', '']
path = os.path.join(ROOT, 'docs', 'cli', 'commands.md')
with open(path, 'w', encoding='utf8', newline='\n') as f:
    f.write('\n'.join(out))
shutil.rmtree(HOME, ignore_errors=True)
print(f'{path}: {len(SECTIONS)} commands')
