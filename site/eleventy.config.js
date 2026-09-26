import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";

export default function (eleventyConfig) {
  // Files that are served as they are.
  eleventyConfig.addPassthroughCopy({ "src/assets": "assets" });
  eleventyConfig.addPassthroughCopy("src/style.css");
  eleventyConfig.addPassthroughCopy("src/script.js");
  eleventyConfig.addPassthroughCopy("src/CNAME");
  eleventyConfig.addPassthroughCopy("src/robots.txt");

  // A text key that does not exist in one language must fail the build, not print nothing.
  eleventyConfig.setNunjucksEnvironmentOptions({ throwOnUndefined: true });

  // Every language needs the same keys as English.
  eleventyConfig.on("eleventy.before", () => {
    const load = (code) => JSON.parse(readFileSync(`src/_data/text/${code}.json`, "utf8"));
    const missing = [];
    const compare = (reference, other, path) => {
      for (const key of Object.keys(reference)) {
        const here = `${path}${key}`;
        if (!(key in other)) {
          missing.push(here);
        } else if (reference[key] && typeof reference[key] === "object" && !Array.isArray(reference[key])) {
          compare(reference[key], other[key], `${here}.`);
        } else if (Array.isArray(reference[key]) && reference[key].length !== other[key].length) {
          missing.push(`${here} (different number of entries)`);
        }
      }
    };
    const english = load("en");
    compare(english, load("de"), "");
    if (missing.length > 0) {
      throw new Error(`de.json differs from en.json: ${missing.join(", ")}`);
    }
  });

  // Nothing unfilled may reach the published pages.
  eleventyConfig.on("eleventy.after", ({ dir }) => {
    const problems = [];
    const walk = (folder) => {
      for (const name of readdirSync(folder)) {
        const path = join(folder, name);
        if (statSync(path).isDirectory()) {
          walk(path);
        } else if (/\.(html|xml)$/.test(name)) {
          const text = readFileSync(path, "utf8");
          if (text.includes("{{") || text.includes("{%")) problems.push(path);
        }
      }
    };
    walk(dir.output);
    if (problems.length > 0) {
      throw new Error(`Unreplaced template markers in: ${problems.join(", ")}`);
    }
  });

  return {
    dir: { input: "src", output: "_site" },
    templateFormats: ["njk"],
  };
}
