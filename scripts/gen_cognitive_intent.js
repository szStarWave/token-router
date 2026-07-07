const fs = require("fs");
const path = require("path");

const OUT = path.join(__dirname, "..", "src", "gateway", "routing", "cognitive_intent.rs");
const LANGS = ["ZH", "EN", "YUE", "JA", "KO"];

// Keep in sync with gen_cognitive_intent.py
const { execFileSync } = require("child_process");
const pyPath = path.join(__dirname, "gen_cognitive_intent.py");

function loadTables() {
  try {
    const out = execFileSync("python", [pyPath], { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });
    if (out.includes("wrote")) return null;
  } catch (_) {}
  // Fallback: require running via embedded minimal loader — parse py with eval of JSON-like extract
  const py = fs.readFileSync(pyPath, "utf8");
  const start = py.indexOf("TABLES:");
  const end = py.indexOf("FUNC_MAP");
  let block = py.slice(py.indexOf("{", start), py.lastIndexOf("}", end) + 1);
  block = block
    .replace(/'/g, '"')
    .replace(/,\s*]/g, "]")
    .replace(/,\s*}/g, "}")
    .replace(/("#)/g, '"$1');
  // Python True/False not present; keys are quoted via replace on single quotes won't work for nested
  return null;
}

// Inline tables (mirrors gen_cognitive_intent.py)
const TABLES = require("./cognitive_intent_tables.json");

const FUNC_MAP = {
  ANALYSIS_INTENT: "contains_analysis_intent",
  DECISION_INTENT: "contains_decision_intent",
  RESEARCH_INTENT: "contains_research_intent",
};

function flatten(clusters) {
  return clusters.flat();
}

function emitArray(name, terms) {
  const lines = [`const ${name}: &[&str] = &[`];
  for (const t of terms) {
    lines.push(`    "${t.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}",`);
  }
  lines.push("];");
  lines.push(`const _: () = assert!(${name}.len() >= 64);`);
  return lines.join("\n");
}

function main() {
  const parts = [
    "//! Cognitive task intent keywords (analysis / decision / research).",
    "",
    "use super::keywords::matches_any_lang;",
    "",
  ];
  for (const [group, langs] of Object.entries(TABLES)) {
    parts.push(`// --- ${group} ---`);
    const refs = [];
    for (const lang of LANGS) {
      const terms = flatten(langs[lang]);
      if (terms.length < 64) throw new Error(`${group}_${lang} has ${terms.length}`);
      const cname = `${group}_${lang}`;
      parts.push(emitArray(cname, terms));
      parts.push("");
      refs.push(cname);
    }
    const fn = FUNC_MAP[group];
    parts.push(`pub fn ${fn}(text: &str) -> bool {`);
    parts.push(`    matches_any_lang(text, &[${refs.map((r) => "&" + r).join(", ")}])`);
    parts.push("}");
    parts.push("");
  }
  parts.push("#[cfg(test)]");
  parts.push("mod tests {");
  parts.push("    use super::*;");
  parts.push("");
  parts.push("    #[test]");
  parts.push('    fn analysis_intent_smoke() {');
  parts.push('        assert!(contains_analysis_intent("分析一下美股股票的情况"));');
  parts.push("    }");
  parts.push("");
  parts.push("    #[test]");
  parts.push('    fn decision_intent_smoke() {');
  parts.push('        assert!(contains_decision_intent("A和B哪个更好，帮我决策"));');
  parts.push("    }");
  parts.push("");
  parts.push("    #[test]");
  parts.push('    fn research_intent_smoke() {');
  parts.push('        assert!(contains_research_intent("帮我研究量子计算前沿"));');
  parts.push("    }");
  parts.push("}");

  fs.writeFileSync(OUT, parts.join("\n") + "\n", "utf8");
  console.log("wrote", OUT);
}

main();
