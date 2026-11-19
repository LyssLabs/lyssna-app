import { useEffect, useState } from "react";
import { ExternalLink, LogOut } from "lucide-react";
import { SettingRow } from "./setting-row";

interface Account {
  id: string;
  username: string;
  role: string;
  reputation: number;
  tier_name: string;
  email_verified: boolean;
}

/** Lyssna account for submitting and reviewing lyrics. Sign-in happens on
 *  lyssna.org in the browser; the tokens stay in the credential store. */
export function LyssnaAccount({
  onCall,
  onNotice,
}: {
  onCall: <T = void>(
    command: string,
    args?: Record<string, unknown>,
  ) => Promise<T | undefined>;
  onNotice: (message: string, error?: boolean) => void;
}) {
  const [account, setAccount] = useState<Account | null>(null);
  const [loading, setLoading] = useState(true);
  const [signingIn, setSigningIn] = useState(false);
  useEffect(() => {
    let active = true;
    void onCall<Account | null>("lyssna_account")
      .then((value) => {
        if (active) setAccount(value ?? null);
      })
      .finally(() => {
        if (active) setLoading(false);
      });
    return () => {
      active = false;
    };
  }, [onCall]);
  const signIn = async () => {
    setSigningIn(true);
    try {
      const found = await onCall<Account>("lyssna_sign_in");
      if (found) {
        setAccount(found);
        onNotice(`Signed in to Lyssna as ${found.username}`);
      }
    } finally {
      setSigningIn(false);
    }
  };
  const signOut = async () => {
    await onCall("lyssna_sign_out");
    setAccount(null);
    onNotice("Signed out of Lyssna");
  };
  const role =
    account && account.role !== "user"
      ? `${account.role[0].toUpperCase()}${account.role.slice(1)} · `
      : "";
  return (
    <SettingRow
      label={account ? account.username : "Lyssna account"}
      help={
        loading
          ? "Checking…"
          : account
            ? `${role}${account.tier_name} · ${account.reputation} reputation${account.email_verified ? "" : " · confirm your email on lyssna.org to submit lyrics"}`
            : signingIn
              ? "Finish signing in in your browser."
              : "Sign in to submit lyrics, review them and get notified when yours are approved."
      }
      keywords="lyssna account sign in log in submit lyrics review profile"
    >
      {account ? (
        <div className="inline-actions">
          <button
            className="pill outline"
            onClick={() =>
              void onCall("open_link", { url: "https://lyssna.org/u/me" })
            }
          >
            Profile
            <ExternalLink size={13} />
          </button>
          <button className="pill outline" onClick={() => void signOut()}>
            <LogOut size={14} />
            Sign out
          </button>
        </div>
      ) : signingIn ? (
        <button
          className="pill outline"
          onClick={() => void onCall("lyssna_cancel_sign_in")}
        >
          Cancel
        </button>
      ) : (
        <button
          className="pill"
          disabled={loading}
          onClick={() => void signIn()}
        >
          Sign in
          <ExternalLink size={13} />
        </button>
      )}
    </SettingRow>
  );
}
