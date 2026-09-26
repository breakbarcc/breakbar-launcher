import { readFileSync } from "node:fs";

// The version written on the pages. The deploy workflow passes the newest release in SITE_VERSION;
// for a local preview it falls back to the version in the workspace's Cargo.toml.
function cargoVersion() {
  const toml = readFileSync(new URL("../../../Cargo.toml", import.meta.url), "utf8");
  const match = toml.match(/\[workspace\.package\][^[]*?\nversion\s*=\s*"([^"]+)"/);
  if (!match) throw new Error("No workspace version found in Cargo.toml");
  return match[1];
}

const repo = "https://github.com/breakbarcc/breakbar-launcher";

export default {
  origin: "https://launcher.breakbar.cc",
  version: (process.env.SITE_VERSION || cargoVersion()).replace(/^v/, ""),
  urls: {
    repo,
    download: `${repo}/releases/latest/download/breakbar-launcher.exe`,
    checksum: `${repo}/releases/latest/download/breakbar-launcher.exe.sha256`,
    releases: `${repo}/releases`,
    changelog: `${repo}/blob/main/CHANGELOG.md`,
    license: `${repo}/blob/main/LICENSE`,
    issues: `${repo}/issues/new`,
    main: "https://www.breakbar.cc/",
    // The legal pages of the whole breakbar.cc project (one operator); this site has none of its own.
    imprint: "https://www.breakbar.cc/impressum",
    privacy: "https://www.breakbar.cc/datenschutz",
    slint: "https://slint.dev",
  },
};
