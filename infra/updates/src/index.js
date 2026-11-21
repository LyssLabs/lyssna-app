// @ts-check

const artifact = /^releases\/\d+\.\d+\.\d+\/(windows-x86_64\/(lyssna\.exe|spotiflex\.exe|source\.zip)|macos-universal\/Lyssna\.app\.zip)$/;
// Copies from before the rename read the old channel and only accept spotiflex.exe.
const channels = new Map([
  ["channels/stable/lyssna-windows-x86_64.json", { target: "windows-x86_64", file: "lyssna.exe" }],
  ["channels/stable/windows-x86_64.json", { target: "windows-x86_64", file: "spotiflex.exe" }],
  ["channels/stable/lyssna-macos-universal.json", { target: "macos-universal", file: "Lyssna.app.zip" }],
]);

const windows = { channel: "channels/stable/lyssna-windows-x86_64.json", filename: "lyssna_setup.exe", type: "application/octet-stream" };
const macos = { channel: "channels/stable/lyssna-macos-universal.json", filename: "Lyssna-macOS.zip", type: "application/zip" };
// Fixed links that always download the current stable release. The
// lyssna.exe paths stay for links already shared.
const latest = new Map([
  ["latest/lyssna_setup.exe", windows],
  ["lyssna_setup.exe", windows],
  ["latest/lyssna.exe", windows],
  ["lyssna.exe", windows],
  ["latest/Lyssna-macOS.zip", macos],
  ["Lyssna-macOS.zip", macos],
]);

// Installed Windows copies fetch the versioned exe to update themselves; every
// other full download is someone getting Lyssna.
/** @param {string} key */
function downloadKind(key) {
  const [, version, target, file] = key.split("/");
  if (file === "source.zip") return null;
  const platform = target === "macos-universal" ? "macos" : "windows";
  return { version, platform, kind: platform === "windows" ? "update" : "download" };
}

/** @param {Env & { STATS?: D1Database }} env @param {ExecutionContext | undefined} ctx @param {{ version: string, platform: string, kind: string } | null} entry */
function count(env, ctx, entry) {
  if (!entry || !env.STATS || !ctx) return;
  ctx.waitUntil(
    env.STATS.prepare(
      `INSERT INTO downloads (day, platform, kind, version, count) VALUES (?1, ?2, ?3, ?4, 1)
       ON CONFLICT (day, platform, kind, version) DO UPDATE SET count = count + 1`,
    )
      .bind(new Date().toISOString().slice(0, 10), entry.platform, entry.kind, entry.version)
      .run()
      .catch(() => {}),
  );
}

/** @param {Env & { STATS?: D1Database }} env */
async function stats(env) {
  const headers = { "Access-Control-Allow-Origin": "*", "Cache-Control": "public, max-age=300" };
  if (!env.STATS) return Response.json({ since: null, totals: [], versions: [], days: [] }, { headers });
  const since = new Date(Date.now() - 29 * 86_400_000).toISOString().slice(0, 10);
  const [first, totals, versions, days] = await env.STATS.batch([
    env.STATS.prepare("SELECT MIN(day) AS day FROM downloads"),
    env.STATS.prepare("SELECT platform, kind, SUM(count) AS count FROM downloads GROUP BY platform, kind"),
    env.STATS.prepare("SELECT version, platform, kind, SUM(count) AS count FROM downloads GROUP BY version, platform, kind"),
    env.STATS.prepare("SELECT day, platform, kind, SUM(count) AS count FROM downloads WHERE day >= ?1 GROUP BY day, platform, kind ORDER BY day").bind(since),
  ]);
  return Response.json(
    { since: /** @type {{ day?: string } | undefined} */ (first.results[0])?.day ?? null, totals: totals.results, versions: versions.results, days: days.results },
    { headers },
  );
}

/** @param {string} value */
function decode(value) {
  return Uint8Array.from(atob(value), (character) => character.charCodeAt(0));
}

/** @param {Request} request @param {string} token */
async function authorized(request, token) {
  const supplied = request.headers.get("Authorization")?.replace(/^Bearer /, "") ?? "";
  if (!token || supplied.length > 256) return false;
  const encoder = new TextEncoder();
  const [expected, actual] = await Promise.all([
    crypto.subtle.digest("SHA-256", encoder.encode(token)),
    crypto.subtle.digest("SHA-256", encoder.encode(supplied)),
  ]);
  return crypto.subtle.timingSafeEqual(expected, actual);
}

/** @param {Request} request @param {Env} env @param {string} key */
async function publish(request, env, key) {
  if (!(await authorized(request, env.PUBLISH_TOKEN))) return new Response("Unauthorized", { status: 401 });
  const size = Number(request.headers.get("Content-Length"));
  const channel = channels.get(key);
  const manifest = channel !== undefined;
  if (!Number.isSafeInteger(size) || size <= 0 || size > (manifest ? 65536 : 512 * 1024 * 1024)) {
    return new Response("Invalid content length", { status: 413 });
  }
  if (!request.body) return new Response("Body required", { status: 400 });
  if (manifest) {
    try {
      const text = await request.text();
      if (new TextEncoder().encode(text).length > 65536) return new Response("Manifest too large", { status: 413 });
      const envelope = JSON.parse(text);
      const payload = decode(envelope.payload);
      const verificationKey = await crypto.subtle.importKey("raw", decode(env.UPDATE_PUBLIC_KEY), "Ed25519", false, ["verify"]);
      if (!(await crypto.subtle.verify("Ed25519", verificationKey, decode(envelope.signature), payload))) return new Response("Invalid signature", { status: 400 });
      const release = JSON.parse(new TextDecoder().decode(payload));
      if (!/^\d+\.\d+\.\d+$/.test(release.version) || release.target !== channel.target || release.artifact !== `releases/${release.version}/${channel.target}/${channel.file}` || !/^[a-fA-F0-9]{64}$/.test(release.sha256)) return new Response("Invalid release", { status: 400 });
      const object = await env.RELEASES.head(release.artifact);
      if (!object || object.size !== release.size) return new Response("Upload executable first", { status: 409 });
      await env.RELEASES.put(key, text, { httpMetadata: { contentType: "application/json", cacheControl: "public, max-age=60" } });
    } catch {
      return new Response("Invalid signed manifest", { status: 400 });
    }
  } else {
    if (await env.RELEASES.head(key)) return new Response("Version already exists", { status: 409 });
    const stored = await env.RELEASES.put(key, request.body, { onlyIf: { etagDoesNotMatch: "*" }, httpMetadata: { contentType: key.endsWith(".zip") ? "application/zip" : "application/octet-stream", cacheControl: "public, max-age=31536000, immutable" } });
    if (!stored) return new Response("Version already exists", { status: 409 });
  }
  return new Response(null, { status: 201 });
}

/** The file a stable channel names, checked against its signature again so
 * a bad manifest in the bucket never decides what is served.
 * @param {Request} request @param {Env} env @param {{ channel: string, filename: string, type: string }} link @param {ExecutionContext} [ctx] */
async function latestRelease(request, env, link, ctx) {
  const missing = () => new Response("Not found", { status: 404, headers: { "Cache-Control": "no-store" } });
  const expected = channels.get(link.channel);
  const channel = await env.RELEASES.get(link.channel);
  if (!channel) return missing();
  let artifactKey;
  try {
    const envelope = JSON.parse(await new Response(channel.body).text());
    const payload = decode(envelope.payload);
    const verificationKey = await crypto.subtle.importKey("raw", decode(env.UPDATE_PUBLIC_KEY), "Ed25519", false, ["verify"]);
    if (!(await crypto.subtle.verify("Ed25519", verificationKey, decode(envelope.signature), payload))) return missing();
    artifactKey = JSON.parse(new TextDecoder().decode(payload)).artifact;
  } catch {
    return missing();
  }
  if (typeof artifactKey !== "string" || !artifact.test(artifactKey) || !expected || !artifactKey.endsWith(`/${expected.target}/${expected.file}`)) return missing();
  const object = request.method === "HEAD" ? await env.RELEASES.head(artifactKey) : await env.RELEASES.get(artifactKey);
  if (!object) return missing();
  const headers = new Headers({
    "X-Content-Type-Options": "nosniff",
    "Content-Security-Policy": "default-src 'none'",
    "Content-Type": link.type,
    "Content-Disposition": `attachment; filename="${link.filename}"`,
    "Content-Length": String(object.size),
    ETag: object.httpEtag,
    // Short, so a new release reaches the fixed link within a minute.
    "Cache-Control": "public, max-age=60",
    "X-Lyssna-Version": artifactKey.split("/")[1],
  });
  if (request.headers.get("If-None-Match") === object.httpEtag) return new Response(null, { status: 304, headers });
  const entry = downloadKind(artifactKey);
  if (request.method === "GET" && entry) count(env, ctx, { ...entry, kind: "download" });
  return new Response("body" in object && object.body instanceof ReadableStream ? object.body : null, { headers });
}

export default {
  /** @param {Request} request @param {Env} env @param {ExecutionContext} [ctx] */
  async fetch(request, env, ctx) {
    const url = new URL(request.url);
    if (url.pathname === "/stats.json" && request.method === "GET") return stats(env);
    if (url.pathname === "/health" && request.method === "GET") return Response.json({ service: "spotiflex-updates", status: "ok" });
    const fixed = latest.get(url.pathname.slice(1));
    if (fixed) {
      if (!["GET", "HEAD"].includes(request.method)) return new Response("Method not allowed", { status: 405, headers: { Allow: "GET, HEAD" } });
      return latestRelease(request, env, fixed, ctx);
    }
    const publishing = url.pathname.startsWith("/publish/");
    const key = url.pathname.slice(publishing ? "/publish/".length : 1);
    if (!artifact.test(key) && !channels.has(key)) return new Response("Not found", { status: 404 });
    if (publishing) {
      if (request.method !== "PUT") return new Response("Method not allowed", { status: 405, headers: { Allow: "PUT" } });
      return publish(request, env, key);
    }
    if (!["GET", "HEAD"].includes(request.method)) return new Response("Method not allowed", { status: 405, headers: { Allow: "GET, HEAD" } });
    const object = request.method === "HEAD" ? await env.RELEASES.head(key) : await env.RELEASES.get(key);
    if (!object) return new Response("Not found", { status: 404, headers: { "Cache-Control": "no-store" } });
    const headers = new Headers({ "X-Content-Type-Options": "nosniff", "Content-Security-Policy": "default-src 'none'" });
    object.writeHttpMetadata(headers);
    headers.set("ETag", object.httpEtag);
    headers.set("Content-Length", String(object.size));
    headers.set("Cache-Control", channels.has(key) ? "public, max-age=60" : "public, max-age=31536000, immutable");
    if (request.headers.get("If-None-Match") === object.httpEtag) return new Response(null, { status: 304, headers });
    if (request.method === "GET" && !channels.has(key)) count(env, ctx, downloadKind(key));
    return new Response("body" in object && object.body instanceof ReadableStream ? object.body : null, { headers });
  },
};
