import { rpc } from "./bridge";
import type { Settings } from "./types";
import { isRecord, mergeLyrics, standalonePreferences, type SettingsRecord, type SettingsTransfer, type TransferLyrics } from "./transfer-format";

type NativeExport = { settings: Settings; lyrics: TransferLyrics[]; preserved: unknown };

function windowPreferences(): Record<string, string> {
  const preferences: Record<string, string> = {};
  for (let index = 0; index < localStorage.length; index++) {
    const key = localStorage.key(index);
    if (key?.startsWith("lyssna.")) preferences[key] = localStorage.getItem(key)!;
  }
  return preferences;
}

function fromNative(data: NativeExport, storage?: Record<string, string>): SettingsTransfer {
  const preserved = isRecord(data.preserved) ? data.preserved as Partial<SettingsTransfer> : {};
  return {
    format: "lyssna-settings", version: 1, source: "standalone", exportedAt: new Date().toISOString(),
    plugin: preserved.plugin,
    standalone: { settings: { ...preserved.standalone?.settings, ...data.settings }, storage: storage ?? preserved.standalone?.storage },
    lyrics: data.lyrics,
  };
}

export async function exportStandaloneSettings(): Promise<SettingsTransfer> {
  return fromNative(await rpc<NativeExport>("export_settings_transfer"), windowPreferences());
}

export async function previousStandaloneBackup(): Promise<SettingsTransfer | undefined> {
  const data = await rpc<NativeExport | null>("previous_settings_backup");
  return data ? fromNative(data) : undefined;
}

export function hasStandaloneBackup(): Promise<boolean> {
  return rpc<boolean>("has_settings_backup");
}

export function prepareStandaloneImport(file: SettingsTransfer) {
  const settings = standalonePreferences(file);
  // A preserved legacy file without a URI can still be restored by its original key.
  return { file, settings, lyrics: mergeLyrics([], file.lyrics) };
}

export async function importStandaloneSettings(plan: ReturnType<typeof prepareStandaloneImport>, current: Settings): Promise<string> {
  const previous = windowPreferences();
  const incoming = plan.file.standalone?.storage ?? {};
  const changed = Object.keys(incoming);
  const settings: SettingsRecord = { ...current, ...plan.settings };
  // Offsets are keyed by track; songs missing from an import keep their own offsets.
  if (isRecord(plan.settings.lyric_song_offsets)) settings.lyric_song_offsets = { ...current.lyric_song_offsets, ...plan.settings.lyric_song_offsets };
  try {
    for (const key of changed) localStorage.setItem(key, incoming[key]);
    const count = await rpc<number>("import_settings_transfer", {
      settings, lyrics: plan.lyrics, preserved: { ...plan.file, lyrics: [] }, windowPreferences: previous,
    });
    return `Imported ${Object.keys(plan.settings).length} settings and ${count} lyric files. Previous backup is available below. Window and editor preferences take effect when reopened.`;
  } catch (error) {
    let recovered = true;
    for (const key of changed) {
      try {
        if (previous[key] === undefined) localStorage.removeItem(key);
        else localStorage.setItem(key, previous[key]);
      } catch { recovered = false; }
    }
    if (!recovered) throw new Error("Import failed and window preferences could not be fully restored. Download the previous backup before retrying.");
    throw error;
  }
}
