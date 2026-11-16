import { useEffect, useRef, useState, type ReactNode } from "react";
import { ExternalLink } from "lucide-react";

export type ProfileAction =
  | "account"
  | "profile"
  | "recents"
  | "settings"
  | "sign-out"
  | "spotify-sign-in"
  | "ytmusic-sign-out";

/** Avatar button with Spotify's account dropdown. */
export function ProfileMenu({
  name,
  image,
  hidden = [],
  spotify = true,
  ytmusic = false,
  onAction,
}: {
  name: string;
  image?: string;
  spotify?: boolean;
  ytmusic?: boolean;
  /** Items removed through Spotify+; Settings always stays. */
  hidden?: ProfileAction[];
  onAction: (action: ProfileAction) => void;
}) {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const outside = (event: PointerEvent) => {
      if (!root.current?.contains(event.target as Node)) setOpen(false);
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };
    document.addEventListener("pointerdown", outside);
    document.addEventListener("keydown", escape);
    return () => {
      document.removeEventListener("pointerdown", outside);
      document.removeEventListener("keydown", escape);
    };
  }, [open]);
  const item = (action: ProfileAction, label: ReactNode, end?: ReactNode) =>
    !hidden.includes(action) && (
      <button
        role="menuitem"
        onClick={() => {
          setOpen(false);
          onAction(action);
        }}
      >
        <span>{label}</span>
        {end}
      </button>
    );
  return (
    <div className="profile-menu-root" ref={root}>
      <button
        className={`avatar ${open ? "open" : ""}`}
        title={name}
        aria-label="Account menu"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
      >
        {image ? (
          <img src={image} alt="Your Spotify profile" />
        ) : (
          name.slice(0, 1).toUpperCase()
        )}
      </button>
      {open && (
        <div className="profile-menu" role="menu" aria-label="Account">
          {spotify ? (
            <>
              {item("account", "Account", <ExternalLink size={17} />)}
              {item("profile", "Profile")}
              {item("recents", "Recents")}
            </>
          ) : (
            item("spotify-sign-in", "Sign in with Spotify")
          )}
          {item("settings", "Settings")}
          {(ytmusic || (spotify && !hidden.includes("sign-out"))) && <hr />}
          {ytmusic && item("ytmusic-sign-out", "Log out of YouTube Music")}
          {spotify &&
            item("sign-out", ytmusic ? "Log out of Spotify" : "Log out")}
        </div>
      )}
    </div>
  );
}
