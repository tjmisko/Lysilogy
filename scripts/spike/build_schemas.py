"""Generate the paper-artifacts schemas in prompts/.

Strict mode requires every property in `required`, `additionalProperties:
false`, and nulls only through type unions, which is error-prone by hand.
Edit this file and run: python3 -B scripts/spike/build_schemas.py prompts

The core schema carries the fields every extraction needs. Each optional
feature is its own schema so it can be requested on its own, later, without
re-running the core pass.
"""
import json
import sys

out = sys.argv[1] if len(sys.argv) > 1 else "prompts"

def obj(desc=None, **props):
    o = {"type": "object", "additionalProperties": False, "required": list(props), "properties": props}
    if desc: o["description"] = desc
    return o
def s(desc=None, null=False):
    o = {"type": ["string", "null"] if null else "string"}
    if desc: o["description"] = desc
    return o
def i(desc=None, null=False):
    o = {"type": ["integer", "null"] if null else "integer"}
    if desc: o["description"] = desc
    return o
def arr(items, desc=None):
    o = {"type": "array", "items": items}
    if desc: o["description"] = desc
    return o
def enum(values, desc=None):
    o = {"type": "string", "enum": values}
    if desc: o["description"] = desc
    return o

evidence = obj("A verbatim passage from the paper and the page it is on.",
    quote=s("Copied exactly from the page text, 8 to 300 characters."),
    page=i("1-based page number."))
evidence_list = arr(evidence, "Verbatim support. Empty only when the item is a model judgment.")
confidence = enum(["high", "medium", "low"],
    "high: the paper states it directly in the quoted passage. medium: the paper says it, but not in one place. "
    "low: you had to infer it, or the text was hard to read.")
position_item = obj(
    reference_key=s("The in-text key exactly as the paper prints it, e.g. 'Smith and Jones (2004)' or '12'; null if you cannot match it to the bibliography. Never make one up.", null=True),
    cited_work=s("One specific cited work, e.g. 'Smith et al. 2020'. Name a body of literature in `why`, not here."),
    why=s("One plain-language line on the relation."),
    evidence=evidence_list, confidence=confidence)
relation_kinds = ["background", "uses_method", "uses_data", "extends", "supports", "contrasts", "critiques", "compares", "other"]

section_kinds = ["abstract", "background", "research_question", "theory", "methods",
                 "data", "results", "discussion", "limitations", "conclusion",
                 "references", "appendix", "other"]
section_families = ["context", "question", "method", "evidence", "interpretation", "caveat", "reference"]

# The fields, gathered per feature so the core schema and each feature schema
# are assembled from the same definitions.
identity = obj(
    title=s("As printed, without line-break hyphens."),
    subtitle=s(null=True),
    authors=arr(obj(name=s("As printed, without footnote marks."),
                    affiliation=s("The affiliation printed next to the name, else null.", null=True)),
                "All authors, in printed order."),
    year=i(null=True), venue=s(null=True),
    identifiers=obj(doi=s(null=True), arxiv=s("arXiv identifier without the 'arXiv:' prefix.", null=True)),
    publication_state=enum(["preprint", "published", "unknown"]),
    evidence=evidence_list)
# The reason comes first so the choice follows from it rather than the reason
# rationalising a label already written.
paper_type = obj(
    reason=s("One line on what the paper mainly does, written before choosing the value."),
    value=enum(["survey", "method", "empirical", "theory", "dataset", "position", "replication", "resource"],
        "survey: mainly reviews others' work; a paper with new theorems, methods or results is not a survey. "
        "method: proposes a new method, model or algorithm. empirical: tests a question on data or experiments. "
        "theory: derives results formally. dataset: introduces a dataset or benchmark. "
        "position: argues for a view or research agenda, including lectures and essays. "
        "replication: re-tests a previously published result; a reprint or corrected edition is not a replication. "
        "resource: software, a tool or another artefact for others to use."))
abstract = obj(text=s("The authors' own abstract copied verbatim from the page, lines within a paragraph joined with single spaces and paragraphs separated by a blank line; never a summary, a paraphrase or the opening of the introduction. Null if the paper prints no abstract.", null=True),
             page=i(null=True))
thesis = obj(text=s("One plain sentence stating what the paper finds or shows, not what it is about."),
             evidence=evidence_list, confidence=confidence)
card_core = obj(
    thesis=thesis,
    position=obj(predecessors=arr(position_item, "Work this paper builds on."),
                 targets=arr(position_item, "Work it contrasts with or critiques."),
                 replications=arr(position_item, "Earlier work whose result this paper re-tests.")))
question = obj(text=s("The research question or hypothesis."), evidence=evidence_list)
outline_roles = ["front_matter", "main", "back_matter", "references", "appendix"]
outline = arr(obj(
    number=s("The heading's printed number or letter exactly as printed, e.g. '2', '2.1', 'IV.', 'A.'; null when the heading is unnumbered.", null=True),
    title=s("The heading text exactly as printed, without its number."),
    level=i("1 for a top-level heading, 2 for a subsection, 3 below that."),
    page=i("The 1-based PDF page the heading is printed on."),
    role=enum(outline_roles, "front_matter: abstract, keywords and similar before the main text; main: the paper's argument; back_matter: acknowledgments, funding, author notes, declarations; references: the bibliography; appendix: appendices and supplementary material.")),
    "The paper's own headings as printed, in reading order. Empty when the paper prints no headings.")

# The stored logical unit. A unit grouped from the outline names its range of
# outline entries and has its pages and source span derived by the
# application; a unit made without an outline has null outline indices and
# gives its own pages and span.
unit_title = s("Written after the question: a short title by role. Setup and method units name what they set up; result and "
               "interpretation units state the finding. Not a breadcrumb of the paper's own headings, and not two topics joined by 'and'.")
unit_summary = s("One or two sentences that synthesize the whole unit.")
unit_question = s("Write this first: the one question every part of this unit helps answer. If you cannot write one question, change the range.")
unit_pages = obj(start=i("First 1-based PDF page."), end=i("Last 1-based PDF page."))
unit_span = obj(start_text=s("A short verbatim phrase from where the unit begins."), start_page=i(),
                end_text=s("A short verbatim phrase from where the unit ends."), end_page=i())
# Properties are in the order the model should decide them: the extent, then
# the question it answers, then the summary, and the title last.
sections = arr(obj(
    outline_first=i("Index of the first outline entry in the unit; null when no outline was given.", null=True),
    outline_last=i("Index of the last outline entry in the unit; null when no outline was given.", null=True),
    pages=unit_pages,
    source_span=unit_span,
    question=unit_question,
    summary=unit_summary,
    title=unit_title,
    kind=enum(section_kinds),
    family=enum(section_families)),
    "Reading units that each answer one question, usually one to five pages of content. "
    "Increasing PDF order, non-overlapping spans, covering the main text.")
# What the model is asked for when the paper has a usable outline: ranges of
# outline entries only. Pages and spans are derived from the outline.
sections_grouped = arr(obj(
    outline_first=i("Index of the first outline entry in this unit."),
    outline_last=i("Index of the last outline entry in this unit; equal to outline_first for a one-entry unit."),
    question=unit_question,
    summary=unit_summary,
    title=unit_title,
    kind=enum(section_kinds),
    family=enum(section_families)),
    "Reading units as contiguous ranges of the given outline entries, in order, without gaps or overlaps. "
    "Group headings that answer the same question; a printed section that is already one idea may be its own unit.")
references = arr(obj(key=s("The in-text marker, e.g. '12' or 'Smith et al., 2020'."),
                   title=s(null=True), authors=arr(s()), year=i(null=True), venue=s(null=True), doi=s(null=True),
                   page=i()), "Every bibliography entry, in order.")
citations = arr(obj(reference_key=s(), page=i(), context=s("The citing sentence, verbatim.")))
relations = arr(obj(reference_key=s(), relation=enum(relation_kinds), why=s(), quote=s("Verbatim."), page=i(),
                  confidence=confidence))

# Optional features.
supercut = arr(obj(kind=enum(["source", "connector"], "source: the paper's own sentences, verbatim. connector: your short bridging text."),
                 text=s(), page=i("Required for source segments; null for connectors.", null=True)),
             "The argument as ordered segments. Length and quote density follow the paper's importance and prose quality.")
claims = arr(obj(id=s("c1, c2, ..."), statement=s(), support=s("What supports it, in a line."),
               strength=enum(["strong", "moderate", "weak", "speculative"]),
               evidence_refs=arr(s(), "Object labels, numbers, or section names that support it."),
               evidence=evidence_list, confidence=confidence))
method = obj(setting=s(null=True), data=s(null=True), procedure=s(null=True), assumptions=arr(s()),
           evidence=evidence_list, confidence=confidence)
key_numbers = arr(obj(value=s("As printed, with units."), quantity=s(), context=s(),
                    evidence=evidence_list, confidence=confidence))
limitations = arr(obj(text=s(), stated_by=enum(["authors", "model"]), evidence=evidence_list, confidence=confidence))
open_questions = arr(obj(text=s(), stated_by=enum(["authors", "model"]), evidence=evidence_list, confidence=confidence))
objects = arr(obj(kind=enum(["figure", "table", "equation", "algorithm"]),
                label=s("As printed, e.g. 'Figure 3', 'Table 1', '(12)'.", null=True),
                number=s("The number alone, e.g. '3', '1', '12', 'A.2'.", null=True),
                page=i(), caption=s("The printed caption, copied verbatim. Null for equations and for any object without a printed caption; never write one.", null=True),
                latex=s("Equations: LaTeX read from the page image. Null otherwise.", null=True),
                referenced_on_pages=arr(i()), confidence=confidence),
            "Every figure, table, and numbered displayed equation of the requested kinds; also unnumbered displayed equations that matter.")
notation = arr(obj(symbol=s("LaTeX."), meaning=s(), page=i(null=True), evidence=evidence_list, confidence=confidence))
glossary = arr(obj(term=s(), definition=s(), evidence=evidence_list, confidence=confidence))
headline_results = arr(obj(statement=s(), carrier=s("The object label or number that carries the result."),
                         evidence=evidence_list, confidence=confidence))
read_signal = obj(judgment=enum(["read", "skim", "glance", "summary_enough"]),
                  reason=s("Model judgment; say why."))

# The "why this paper matters" context, written after the core fields from the
# paper, the core draft and a dossier of neighbouring works. Every claim says
# where it comes from, so the reader can tell the paper's own words from the
# dossier's records and from the model's unverified background knowledge.
context_basis = enum(["paper", "dossier", "background"],
    "paper: the paper itself says this, and `evidence` quotes it. dossier: a record in the neighbour dossier supports it, "
    "and `work_ids` names the record. background: your own knowledge of the field; the reader sees it labelled unverified.")
context_claim = obj(
    text=s("One claim, in a sentence or two of plain prose."),
    basis=context_basis,
    evidence=arr(evidence, "For a paper claim: one or two verbatim quotes that state it. Empty for other claims."),
    work_ids=arr(s(), "For a dossier claim: the ids of the dossier records that support it, exactly as given. Empty for other claims."))
neighbour_relations = ["predecessor", "successor", "critique", "replication", "survey", "parallel"]
read_alongside_basis = enum(["paper", "dossier", "background"],
    "dossier: the work is in the dossier and `work_id` is set. background: you know the work yourself; `work_id` is null. "
    "paper: the paper itself cites it.")
read_alongside = arr(obj(
    work_id=s("The dossier id of the work, exactly as given; null when the work is not in the dossier.", null=True),
    title=s("The work's exact title. If you know the work but not its exact title, give the authors and year and describe it in `why` instead of guessing."),
    authors=s("Short author string, e.g. 'Card' or 'Vaswani et al.'.", null=True),
    year=i(null=True),
    relation=enum(neighbour_relations, "predecessor: came before and this paper builds on or answers it. successor: came after and builds on this paper. "
                  "critique: disputes this paper, or this paper disputes it. replication: reruns or re-tests the result. "
                  "survey: an overview or textbook treatment of the question. parallel: independent work on the same question at about the same time."),
    why=s("One line on what the reader gets from reading it next to this paper."),
    basis=read_alongside_basis),
    "Up to six neighbouring works worth reading next to this one; only works you could cite accurately. Two right works beat five with one invented.")
# The lead comes last: it distils the parts, so the model works them out
# first. The reader shows the lead first whatever the JSON order.
context = obj(
    before=arr(context_claim, "The state of the question before the paper: the specific prior results, methods and debates it entered."),
    contribution=arr(context_claim, "What is new relative to `before`: the specific result, method or reframing this paper added."),
    after=arr(context_claim, "What happened after: who built on it, who disputed it, how it is used now. Empty when you do not know."),
    reception_note=s("When you do not know how the paper was received, say so here in one sentence (for example that it is too recent); otherwise null.", null=True),
    read_alongside=read_alongside,
    lead=arr(context_claim, "Written last: two to four sentences that distil before, contribution and after for a reader from another field. "
             "Say what changed, or, when reception is unknown, what the paper attempts that its predecessors did not."))

CORE = {
    "identity": identity,
    "type": paper_type,
    "abstract": abstract,
    "card": card_core,
    "question": question,
    "outline": outline,
    "sections": sections,
    "references": references,
    "citations": citations,
    "relations": relations,
}
FEATURES = {
    "objects": {"objects": objects},
    "supercut": {"supercut": supercut},
    "analysis": {"claims": claims, "method": method, "key_numbers": key_numbers,
                 "limitations": limitations, "open_questions": open_questions,
                 "headline_results": headline_results, "read_signal": read_signal},
    "glossary": {"notation": notation, "glossary": glossary},
    "context": {"context": context},
}

full = dict(CORE)
for fields in FEATURES.values():
    full.update(fields)
full["card"] = obj(
    thesis=thesis,
    position=obj(predecessors=arr(position_item, "Work this paper builds on."),
                 targets=arr(position_item, "Work it contrasts with or critiques."),
                 replications=arr(position_item, "Earlier work whose result this paper re-tests.")),
    headline_results=headline_results, read_signal=read_signal)
paper = obj("The paper-alone artifacts of one scientific paper.", **full)

check = obj(
    "Corrections to a draft of paper-artifacts. Only correct or flag; every correction needs verbatim evidence.",
    corrections=arr(obj(
        path=s("JSON Pointer into the draft, e.g. '/identity/title' or '/objects/3/latex'. For add, point at the array, e.g. '/objects'."),
        action=enum(["replace", "remove", "add", "flag"]),
        value_json=s("The new value encoded as a JSON string, for replace and add; null for remove and flag.", null=True),
        reason=s("For a flag: what is wrong and what you would change, in one sentence. Otherwise a short reason."),
        evidence=arr(evidence, "At least one verbatim quote that shows the correction is right; a correction without one is discarded."))))

repair = obj(
    "Repairs for quotes that are not verbatim in the paper.",
    repairs=arr(obj(id=s("The id given for the failing quote."),
                    action=enum(["replace_quote", "withdraw"]),
                    quote=s("The exact text from the page, for replace_quote.", null=True),
                    page=i(null=True))))

core_schema = obj("The fields every extraction needs: identity, abstract, context, outline, sections, references, citations and relations.", **CORE)
analysis_schema = obj("The paper's claims, method, key numbers, limitations, open questions, headline results and read signal.",
                      **FEATURES["analysis"])
glossary_schema = obj("The symbols and terms a newcomer would need.", **FEATURES["glossary"])
supercut_schema = obj("The argument as ordered segments.", **FEATURES["supercut"])
context_schema = obj("Why this paper matters: before, contribution, after, and what to read alongside it.", **FEATURES["context"])
objects_schema = obj(
    "The figures, tables, algorithms and numbered equations printed on the given pages.",
    objects=json.loads(json.dumps(objects)),
)
objects_schema["properties"]["objects"]["description"] = (
    "Every figure, table and algorithm of the requested kinds whose caption is printed on the given pages, "
    "and every numbered displayed equation of the requested kinds whose number is printed on them. "
    "One object per printed label; never a range."
)

# One schema per core field, for the fan-out mode where each field is its own
# request over the same paper prefix.
field_schemas = {
    "identity": obj("The paper's identity.", identity=identity),
    "type": obj("The kind of paper.", type=paper_type),
    "abstract": obj("The paper's abstract.", abstract=abstract),
    "card": obj("The paper's thesis and position.", card=card_core),
    "question": obj("The paper's research question.", question=question),
    "outline": obj("The paper's printed outline.", outline=outline),
    "sections": obj("The paper's reading map.", sections=sections),
    "sections-grouped": obj("The paper's reading map as ranges of its outline.", sections=sections_grouped),
    "references": obj("The paper's bibliography entries.", references=references),
    "citations": obj("The paper's in-text citations.", citations=citations),
    "relations": obj("The paper's relations to cited works.", relations=relations),
}

for name, schema in [("paper-artifacts", paper), ("paper-core", core_schema),
                     ("paper-objects", objects_schema), ("paper-supercut", supercut_schema),
                     ("paper-analysis", analysis_schema), ("paper-glossary", glossary_schema),
                     ("paper-context", context_schema),
                     ("paper-artifacts-check", check), ("paper-artifacts-repair", repair),
                     *[(f"paper-field-{name}", schema) for name, schema in field_schemas.items()]]:
    with open(f"{out}/{name}.schema.json", "w") as f:
        json.dump(schema, f, indent=2)
        f.write("\n")
