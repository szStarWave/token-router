const fs = require("fs");
const path = require("path");

const ROOT = path.resolve(__dirname, "..");
const PY = path.join(__dirname, "gen_keywords.py");
const OUT = path.join(ROOT, "src", "gateway", "routing", "keywords.rs");
const LANGS = ["ZH", "EN", "YUE", "JA", "KO"];

const FUNC_MAP = {
  TOOL_ERROR: "tool_result_has_error",
  HARD_INTENT: "contains_hard_intent",
  PLAN_INTENT: "contains_plan_intent",
  EASY_INTENT: "contains_easy_intent",
  REJECT_INTENT: "contains_reject_intent",
  UNCERTAINTY: "response_has_uncertainty",
  SPECIAL_LEXICAL: "contains_special_lexical",
};

function loadTablesFromPy() {
  const py = fs.readFileSync(PY, "utf8");
  const start = py.indexOf("TABLES:");
  const end = py.indexOf("FUNC_MAP");
  if (start < 0 || end < 0) throw new Error("TABLES block not found in gen_keywords.py");
  const braceStart = py.indexOf("{", start);
  const braceEnd = py.lastIndexOf("}", end);
  let block = py.slice(braceStart, braceEnd + 1);
  block = block.replace(/,(\s*[}\]])/g, "$1");
  return JSON.parse(block);
}

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
  const TABLES = loadTablesFromPy();
  const parts = [
    "//! Centralized routing keyword tables (7 groups x 5 languages, >=64 terms each).",
    "",
    "/// Match keywords: ASCII terms are case-insensitive with word boundaries for short",
    "/// tokens; CJK/YUE/JA/KO use raw substring.",
    "pub fn text_matches_keywords(text: &str, keywords: &[&str]) -> bool {",
    "    let lower = text.to_ascii_lowercase();",
    "    keywords.iter().any(|k| {",
    "        if k.is_ascii() {",
    "            ascii_keyword_match(&lower, k)",
    "        } else {",
    "            text.contains(k)",
    "        }",
    "    })",
    "}",
    "",
    "fn ascii_keyword_match(lower: &str, keyword: &str) -> bool {",
    "    let kw = keyword.to_ascii_lowercase();",
    "    if kw.len() <= 4 {",
    "        lower",
    "            .split(|c: char| !c.is_ascii_alphanumeric())",
    "            .any(|word| word == kw)",
    "    } else {",
    "        lower.contains(kw.as_str())",
    "    }",
    "}",
    "",
    "fn matches_any_lang(text: &str, tables: &[&[&str]]) -> bool {",
    "    tables.iter().any(|t| text_matches_keywords(text, t))",
    "}",
    "",
  ];

  const constNames = {};

  for (const [group, langs] of Object.entries(TABLES)) {
    parts.push(`// --- ${group} ---`);
    const tablesForFn = [];
    for (const lang of LANGS) {
      const terms = flatten(langs[lang]);
      if (terms.length < 64) {
        throw new Error(`${group}_${lang} has ${terms.length} terms`);
      }
      const cname = `${group}_${lang}`;
      if (!constNames[group]) constNames[group] = {};
      constNames[group][lang] = cname;
      parts.push(emitArray(cname, terms));
      parts.push("");
      tablesForFn.push(cname);
    }
    const fn = FUNC_MAP[group];
    if (group === "PLAN_INTENT") {
      parts.push(`pub fn ${fn}(text: &str) -> bool {`);
      parts.push(`    let lower = text.to_ascii_lowercase();`);
      parts.push(`    let trimmed = lower.trim();`);
      parts.push(`    (trimmed.starts_with("plan ") || trimmed == "plan")`);
      parts.push(
        `        || matches_any_lang(text, &[${tablesForFn.map((t) => "&" + t).join(", ")}])`
      );
      parts.push("}");
    } else {
      parts.push(`pub fn ${fn}(text: &str) -> bool {`);
      parts.push(
        `    matches_any_lang(text, &[${tablesForFn.map((t) => "&" + t).join(", ")}])`
      );
      parts.push("}");
    }
    parts.push("");
  }

  parts.push("#[cfg(test)]");
  parts.push("mod tests {");
  parts.push("    use super::*;");
  parts.push("");
  parts.push("    #[test]");
  parts.push("    fn keyword_table_minimum_sizes() {");
  for (const langs of Object.values(constNames)) {
    for (const cname of Object.values(langs)) {
      parts.push(`        assert!(${cname}.len() >= 64);`);
    }
  }
  parts.push("    }");
  parts.push("");
  parts.push("    #[test]");
  parts.push("    fn tool_error_smoke() {");
  parts.push('        assert!(tool_result_has_error("Error: command failed"));');
  parts.push('        assert!(tool_result_has_error("失败: exit code 1"));');
  parts.push("    }");
  parts.push("");
  parts.push("    #[test]");
  parts.push("    fn uncertainty_cascade_gate() {");
  parts.push('        assert!(response_has_uncertainty("I\'m not sure about that"));');
  parts.push('        assert!(response_has_uncertainty("わからない"));');
  parts.push('        assert!(response_has_uncertainty("唔确定"));');
  parts.push('        assert!(response_has_uncertainty("아마"));');
  parts.push("    }");
  parts.push("");
  parts.push("    #[test]");
  parts.push("    fn special_lexical_smoke() {");
  parts.push('        assert!(contains_special_lexical("Configure Kubernetes ingress"));');
  parts.push('        assert!(contains_special_lexical("GDPR compliance audit"));');
  parts.push("    }");
  parts.push("}");

  fs.writeFileSync(OUT, parts.join("\n") + "\n", "utf8");
  console.log(`Wrote ${OUT} (${fs.statSync(OUT).size} bytes)`);
}

main();
