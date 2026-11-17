import { useEffect, useRef, useState } from "react";
import type { Settings } from "./types";
import { downloadTransfer, parseTransfer, TRANSFER_LIMIT, type SettingsTransfer } from "./transfer-format";
import { exportStandaloneSettings, hasStandaloneBackup, importStandaloneSettings, prepareStandaloneImport, previousStandaloneBackup } from "./settings-transfer";

export function SettingsTransferControls({ settings }: { settings: Settings }) {
  const input = useRef<HTMLInputElement>(null);
  const [busy, setBusy] = useState(false);
  const [pending, setPending] = useState<ReturnType<typeof prepareStandaloneImport>>();
  const [message, setMessage] = useState("");
  const [error, setError] = useState("");
  const [hasBackup, setHasBackup] = useState(false);
  useEffect(() => { void hasStandaloneBackup().then(setHasBackup).catch(() => {}); }, []);
  const run = async (action: () => Promise<void>) => {
    setBusy(true); setError(""); setMessage("");
    try { await action(); } catch (error) { setError(error instanceof Error ? error.message : String(error)); }
    finally { setBusy(false); }
  };
  const exportFile = async (file: SettingsTransfer | undefined, backup = false) => {
    if (!file) throw new Error("No previous import backup is available.");
    downloadTransfer(file, backup ? "lyssna-before-import" : "lyssna-settings");
    setMessage("Settings and saved lyrics exported.");
  };
  return <div className="settings-transfer" aria-busy={busy}>
    <div className="inline-actions" style={{ flexWrap: "wrap" }}>
      <button className="pill outline" type="button" disabled={busy} onClick={() => void run(async () => exportFile(await exportStandaloneSettings()))}>Export settings</button>
      <button className="pill outline" type="button" disabled={busy} onClick={() => input.current?.click()}>Import settings</button>
      {hasBackup && <button className="pill outline" type="button" disabled={busy} onClick={() => void run(async () => exportFile(await previousStandaloneBackup(), true))}>Download previous backup</button>}
    </div>
    <p>Transfer files can contain saved provider tokens. Keep them private.</p>
    <input ref={input} type="file" accept=".json,.lyssna.json,application/json" hidden onChange={event => {
      const file = event.currentTarget.files?.[0]; event.currentTarget.value = "";
      setPending(undefined);
      if (file) void run(async () => {
        if (file.size > TRANSFER_LIMIT) throw new Error("Settings files must be under 100 MB.");
        setPending(prepareStandaloneImport(parseTransfer(await file.text())));
      });
    }} />
    {pending && <div>
      <p>Import {Object.keys(pending.settings).length} settings and {pending.lyrics.length} lyric files from {pending.file.source === "plugin" ? "Lyssna Lyrics" : "Lyssna Standalone"}? Matching songs are replaced; other saved songs stay. A backup is saved first.</p>
      <p>App-specific options are kept for future exports and applied where supported.</p>
      <div className="inline-actions">
        <button className="pill" type="button" disabled={busy} onClick={() => void run(async () => {
          try { setMessage(await importStandaloneSettings(pending, settings)); setPending(undefined); }
          finally { setHasBackup(await hasStandaloneBackup()); }
        })}>Apply import</button>
        <button className="pill outline" type="button" disabled={busy} onClick={() => setPending(undefined)}>Cancel</button>
      </div>
    </div>}
    {busy && <p role="status">Working…</p>}
    {message && <p role="status">{message}</p>}
    {error && <p role="alert">{error}</p>}
  </div>;
}
