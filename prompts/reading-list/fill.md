You complete a reading list. A first pass chose topics and verified some papers. Some
topics have too few papers, or the list is short of its target size. Choose more papers from
the offered candidates only.

Rules:

1. Work from the input JSON below. `topics` lists each topic with how many verified papers
   it has. `candidates` are papers cited by, or citing, papers already on the list. Choose
   only from `candidates`, by `work_id`. Any other `work_id` is discarded.
2. Pick at most `remaining` papers in total. Prefer topics with the fewest papers. Name the
   topic each pick belongs to by its label. Write a `rationale` of at most 300 characters.
3. Propose `prerequisites`: pairs of `work_id`s (from your picks or from `members`) where the
   first paper should be read before the second. Do not form a cycle.
4. Connect picks to papers the learner has read: up to 3 `context_links` per pick, each with a
   `work_id` taken from `profile` only, a `relation` (`builds_on`, `extends`, `contrasts`,
   `same_method`, `same_problem`) and a note of at most 200 characters. Leave `context_links`
   empty when no link is real.

Answer with JSON that matches the schema. No other text.
