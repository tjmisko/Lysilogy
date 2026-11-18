## The task: why this paper matters

This task asks for judgment rather than extraction. You have the paper, a draft of what the paper says (its thesis, the works it positions itself against, its bibliography), and a dossier of neighbouring works that Lysilogy knows about from its citation graph. Write the `context` field: the account a well-read colleague would give of where this paper sits in its field.

### Who reads it

An intelligent reader from another field. Picture an economist opening a machine-learning paper, or a computer scientist opening a labour-economics paper. They can follow an argument and they read the abstract a moment ago, but they do not know the literature, the names, or the debates. This text is the first thing they see after the abstract, and its job is to tell them why the paper is worth their time and how to read it: what question it entered, what it changed, and what happened next. That is hard to get anywhere else, which is why it is worth doing well.

### What good looks like

Good context is specific. It names the prior results, methods and debates the paper entered, says plainly what the paper added relative to them, and says what changed because of it: who built on it, who disputed it, how it is used now. Where there is disagreement it says so, including when a famous result was later contested or failed to replicate. It is economical: every sentence carries a distinction, a name, a date or a consequence the reader did not have.

Bad context is generic. "A seminal contribution that has influenced much subsequent work" could be pasted onto any well-cited paper, so it tells the reader nothing; say which work, and how. Restating the abstract is also bad, because the reader has just read it. So is hype, and so is a list of topics in place of a claim.

An illustration of the difference, from a field deliberately unlike the paper's so that only its shape carries over: weak is "This landmark trial changed how doctors think about hormone therapy." Strong is "Large observational cohorts had suggested that hormone therapy after menopause lowered the risk of heart disease; this randomized trial found a small increase instead and was stopped early, prescriptions fell sharply, and later reanalyses by age at the start of treatment argued that the harm was concentrated in older women, so the timing question is still argued." Do not copy its shape; most papers have no live controversy, and saying one exists when it does not is a false claim.

### Where each claim comes from

Every claim carries a `basis`, because the reader needs to know what to trust.

- `paper`: the paper itself says it. Quote it in `evidence`, copied exactly from the page text with its page, as everywhere else in this record. The application checks every quote; a claim whose quote cannot be found is shown as unverified.
- `dossier`: a record in the dossier supports it, for example that a later work cites this paper, or when a predecessor was published. Put the record's id in `work_ids`, exactly as written in the dossier. An id that is not in the dossier is removed and the claim is shown as unverified. The dossier records titles, years, authors and citation counts, not what a work argues, so a claim about what a dossier work says beyond its title is usually `background` with the id attached as a pointer.
- Some dossier records also carry a citation label: why this paper cites the work (on works it cites), or why the later work cites this paper (on citing works). Each label names a relation, a reason and a quote that the application checked against the citing text, and says whether a person has reviewed it. You may rely on a label as graph-record evidence: a claim that rests on it (that this paper takes its estimator from a predecessor, or that a later paper disputes this one) is `dossier` with that record's id. Labels marked perfunctory, or saying the passages do not show why, are evidence only that the work was cited, not of what role it played. A label tells you about one citation, not about the field; do not stretch it further.
- `background`: your own knowledge of the field. Use it freely where you actually know something, because much of the value is here, and the reader will see it labelled as unverified model knowledge. Do not dress it up as more certain than it is.

When you do not know how a paper was received, say so in `reception_note` and leave `after` empty or short. "Not known" is useful; a guess about reception presented as fact does real harm.

Your knowledge of the field ends some time before today, and the brief below gives today's date and the paper's year when it is known. For a paper from near or after the point where your knowledge ends, or one you do not recognise, say so in `reception_note`, leave `after` empty, and lead with what the paper attempts and what is new relative to `before`, not with an impact claim. Most papers are not famous; an honest account of an obscure paper's aim and novelty is more useful than invented influence.

### The parts

Work them out in this order, which is also the order of the answer: what came before, what the paper added, what followed, what to read alongside it, and only then the lead that distils them. The reader sees the lead first; you write it last.

- `before`: the state of the question before the paper. Name the prior work, the method that was standard, the puzzle or disagreement it entered; the labels on the works the paper cites show which ones its argument leans on. One to four notes.
- `contribution`: what is new relative to `before`, concretely. This is not the thesis again: it is the difference the paper made. One to three notes.
- `after`: what followed. Prefer the dossier's citing works when they bear on it, and say what kind of follow-up they are where you know it (extension, application, critique, replication, failed replication); a citing work's label often says exactly this. Empty when unknown.
- `read_alongside`: up to six neighbouring works the reader would gain from reading next to this one, each with the one line that says what they get from it and how it relates (predecessor, successor, critique, replication, a survey or textbook treatment, or parallel work on the same question). List only works you could cite accurately: two right works beat five with one invented. If you know a work but not its exact title, give its authors and year and describe it in `why` rather than guessing a title. Prefer works in the dossier and give their ids, because the reader can open or fetch those from Lysilogy; a work you know that is not in the dossier is fine with a null id and `background` basis. Prefer the works a specialist would actually hand a newcomer over the most-cited ones.
- `lead`: two to four sentences, one claim each, that answer "why should I care about this paper?" for the reader above. Say what changed, or, when reception is unknown, what the paper attempts that its predecessors did not. The reader has just read the abstract and the thesis, so a lead claim based only on the paper is usually a restatement: use `paper` in the lead only for something the abstract does not say, such as the paper's own account of what it overturns. Most lead claims rest on `before` and `after`, so their basis is `dossier` or `background`.

If the dossier is empty, the paper is simply not in Lysilogy's citation graph yet; work from the paper and your own knowledge.
