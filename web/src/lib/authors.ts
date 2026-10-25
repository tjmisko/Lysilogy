const suffixes = new Set(["jr", "jr.", "sr", "sr.", "ii", "iii", "iv"]);
const particles = new Set(["van", "von", "de", "der", "den", "del", "della", "di", "da", "du", "la", "le", "ten", "ter", "bin", "al"]);

function lastName(name: string): string {
  const cleaned = name.replace(/\s+/gu, " ").trim();
  if (cleaned === "") return "";
  // "Last, First" keeps everything before the comma, whatever the first name.
  const comma = cleaned.indexOf(",");
  if (comma > 0) return cleaned.slice(0, comma).trim();
  const words = cleaned.split(" ");
  while (words.length > 1 && suffixes.has((words.at(-1) ?? "").toLocaleLowerCase())) words.pop();
  let start = words.length - 1;
  while (start > 0 && particles.has((words[start - 1] ?? "").toLocaleLowerCase())) start -= 1;
  return words.slice(start).join(" ");
}

/** Splits entries that hold several names ("Axsen and Long", "Weij et al."). */
function expand(entry: string): { names: string[]; etAl: boolean } {
  const etAl = /\bet\.? al\.?$/iu.test(entry.trim());
  const body = entry.replace(/,?\s*\bet\.? al\.?$/iu, "");
  // A lone comma is "Last, First"; more than one separates people.
  const separator = /\s+(?:and|&)\s+|;|,(?=[^,]*,)/u;
  const pieces = separator.test(body) ? body.split(/\s+(?:and|&)\s+|;|,/u) : [body];
  return { names: pieces.map((piece) => piece.trim()).filter(Boolean), etAl };
}

/** Surnames only: "A", "A and B", or "A et al." for three or more. */
export function authorSurnames(authors: readonly string[]): string {
  const names: string[] = [];
  let etAl = false;
  for (const entry of authors) {
    const expanded = expand(entry);
    etAl ||= expanded.etAl;
    for (const name of expanded.names) {
      const surname = lastName(name);
      if (surname !== "") names.push(surname);
    }
  }
  const [first, second] = names;
  if (first === undefined) return "";
  if (names.length === 1) return etAl ? `${first} et al.` : first;
  if (names.length === 2 && !etAl && second !== undefined) return `${first} and ${second}`;
  return `${first} et al.`;
}
