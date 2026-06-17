# Rafael Web Search Quality Implementation

## Goal

Improve the quality of source material sent to the model before increasing raw result counts or fetched byte limits.

This implements suggestion 3 from the quality review: improve search result quality first. The target is a retrieval pipeline that returns cleaner documents, better-ranked sources, and compact evidence chunks instead of dumping noisy page bodies into the model context.

## Current State

The current flow is:

1. The model calls `web_search`.
2. `ChatToolRuntime::invoke_web_search` calls `WebSearchClient::search`.
3. Search returns title, URL, snippet, source, and date metadata.
4. `enrich_search_response` fetches the first `RAFAEL_CHAT_WEB_SEARCH_FETCH_RESULTS` results.
5. `WebFetcher::fetch` extracts all `body` text for HTML pages.
6. The full fetched text is serialized into the tool result.

Useful parts:

- Public URL validation blocks localhost and private-network targets.
- Fetch byte limits are enforced.
- Search providers are abstracted behind `WebSearchClient`.

Weak parts:

- The first search results are fetched before any local quality ranking.
- HTML extraction grabs broad body text, including nav/sidebar/footer noise.
- Search result snippets and fetched body text are not chunked by relevance.
- Fetched pages are sequential, which increases latency.
- Tool output can be larger than the model can use well.
- Metadata currently records all search result URLs, not only the sources that materially grounded the answer.

## Target Pipeline

Replace "search, fetch first N, dump page text" with:

1. Search for candidate results.
2. Normalize and de-duplicate candidate URLs.
3. Score candidate results using lexical relevance, source quality, and date hints.
4. Fetch the best candidates with bounded concurrency.
5. Extract readable document content using article/main-content heuristics.
6. Split extracted text into query-relevant chunks.
7. Re-rank documents after extraction.
8. Return a compact source pack to the model.

The result should contain fewer tokens but better evidence.

## Data Model

Add these structs to `crates/tools/web/src/lib.rs` or split the crate into modules if the file becomes too large.

```rust
pub struct SearchQualityOptions {
    pub candidate_results: usize,
    pub returned_results: usize,
    pub fetch_results: usize,
    pub max_fetch_bytes: usize,
    pub max_chunks_per_document: usize,
    pub max_chunk_chars: usize,
    pub max_total_chunk_chars: usize,
}

pub struct SearchCandidate {
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub source: Option<String>,
    pub published_at: Option<String>,
    pub provider_rank: usize,
}

pub struct ExtractedDocument {
    pub original_url: String,
    pub final_url: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub published_at: Option<String>,
    pub content_type: Option<String>,
    pub text: String,
    pub bytes: usize,
    pub truncated: bool,
    pub extraction: ExtractionMetadata,
}

pub struct ExtractionMetadata {
    pub method: ExtractionMethod,
    pub text_chars: usize,
    pub link_density: f32,
    pub quality_score: f32,
}

pub enum ExtractionMethod {
    Article,
    Main,
    MarkdownContainer,
    BodyFallback,
    PlainText,
}

pub struct EvidenceChunk {
    pub text: String,
    pub score: f32,
    pub start_char: usize,
    pub end_char: usize,
}

pub struct RankedSourceDocument {
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub source: Option<String>,
    pub published_at: Option<String>,
    pub provider_rank: usize,
    pub retrieval_score: f32,
    pub fetched: Option<RankedFetchedEvidence>,
}

pub struct RankedFetchedEvidence {
    pub final_url: String,
    pub chunks: Vec<EvidenceChunk>,
    pub text_chars: usize,
    pub bytes: usize,
    pub truncated: bool,
    pub extraction: ExtractionMetadata,
}
```

The model-facing JSON should expose only `RankedSourceDocument`, not full `ExtractedDocument.text`.

## Configuration

Keep existing env vars working, but separate candidate count from returned evidence count.

Existing env vars:

- `RAFAEL_CHAT_WEB_SEARCH_MAX_RESULTS`
- `RAFAEL_CHAT_WEB_SEARCH_FETCH_RESULTS`
- `RAFAEL_CHAT_WEB_SEARCH_FETCH_MAX_BYTES`

Add:

- `RAFAEL_CHAT_WEB_SEARCH_CANDIDATE_RESULTS`
  - Default: `10`
  - Clamp: `1..=20`
  - Number of raw provider results to retrieve before local ranking.

- `RAFAEL_CHAT_WEB_SEARCH_RETURN_RESULTS`
  - Default: existing `RAFAEL_CHAT_WEB_SEARCH_MAX_RESULTS`
  - Clamp: `1..=8`
  - Number of documents returned to the model.

- `RAFAEL_CHAT_WEB_SEARCH_CHUNKS_PER_DOCUMENT`
  - Default: `3`
  - Clamp: `1..=6`

- `RAFAEL_CHAT_WEB_SEARCH_CHUNK_CHARS`
  - Default: `900`
  - Clamp: `300..=2000`

- `RAFAEL_CHAT_WEB_SEARCH_TOTAL_CHUNK_CHARS`
  - Default: `9000`
  - Clamp: `2000..=24000`

Backward compatibility:

- If only the existing env vars are set, behavior should remain recognizable.
- `RAFAEL_CHAT_WEB_SEARCH_MAX_RESULTS` remains the model-facing maximum allowed by the tool schema.
- Internally, candidate count can be higher than returned result count.

## Candidate Retrieval

Update `SearchOptions` to distinguish provider count from returned count.

```rust
pub struct SearchOptions {
    pub max_results: usize,
}
```

For this quality pass, keep the public shape but pass `candidate_results` into provider calls. The structured-controls implementation can expand `SearchOptions` later.

Provider behavior:

- Brave: request up to `candidate_results.min(20)`.
- SearXNG: request page 1 and take up to `candidate_results`.

Normalize candidates:

- Trim title, URL, and snippet.
- Drop candidates with empty URL.
- Canonicalize URLs for de-duplication:
  - Lowercase scheme and host.
  - Strip fragment.
  - Remove common tracking query params: `utm_*`, `fbclid`, `gclid`, `mc_cid`, `mc_eid`.
  - Preserve meaningful query params.
- De-duplicate by canonical URL.

## Local Candidate Scoring

Implement deterministic scoring. Do not add an embedding model or LLM reranker in the first pass.

Tokenize query, title, snippet, host, and path:

- Lowercase.
- Split on non-alphanumeric characters.
- Drop tokens shorter than 2 characters.
- Drop a small hardcoded stopword list.

Base score:

- Title token match: `4.0` per unique query token.
- Snippet token match: `2.0` per unique query token.
- URL host/path token match: `1.0` per unique query token.
- Exact phrase in title: `8.0`.
- Exact phrase in snippet: `4.0`.
- Provider rank boost: `1.0 / (rank + 1)`.

Source quality boost:

- Official docs/source repository hosts get a small boost when query looks technical:
  - `docs.*`, `*.readthedocs.io`, `github.com`, `gitlab.com`, `developer.*`, `*.dev`, project-owned domains.
  - Boost: `2.5`
- Package/reference hosts for technical queries:
  - `docs.rs`, `crates.io`, `npmjs.com`, `pypi.org`, `pkg.go.dev`, `developer.mozilla.org`
  - Boost: `3.0`

Penalties:

- Empty snippet: `-2.0`
- URL path contains obvious login/account/cart/share fragments: `-3.0`
- Known low-signal page titles such as "Just a moment", "Attention Required", "Access denied": `-8.0`
- Host is a URL shortener: `-5.0`

These weights should live in a small scoring function with unit tests, not scattered through the fetcher.

## Fetching

Fetch only the top `fetch_results` candidates after candidate scoring.

Use bounded concurrency:

- Add `futures-util` or use `tokio::task::JoinSet`.
- Limit concurrent fetches to `3` by default.
- Preserve final ordering by score after fetches complete.

Redirect handling:

Current fetcher disables redirects. Keep the SSRF safety property, but allow safe redirects manually.

Implement:

- `RAFAEL_CHAT_WEB_FETCH_MAX_REDIRECTS`
  - Default: `3`
  - Clamp: `0..=8`
- When a response is `301`, `302`, `303`, `307`, or `308`, read `Location`.
- Resolve relative `Location` against current URL.
- Re-run `validate_public_url` on the redirect target.
- Reject redirects to localhost, private IPs, bare local names, or unsupported schemes.
- Reject redirect loops.

This improves real-world fetch success while preserving the existing private-network boundary.

## HTML Extraction

Replace body-only extraction with a ranked extraction strategy.

For HTML:

1. Parse with `scraper::Html`.
2. Extract metadata:
   - `<title>`
   - `meta[name=description]`
   - `meta[property=og:description]`
   - `meta[property=article:published_time]`
   - `time[datetime]`
3. Evaluate candidate selectors in order:
   - `article`
   - `main`
   - `[role="main"]`
   - `.markdown-body`
   - `.docs-content`
   - `.documentation`
   - `.content`
   - `.post`
   - `.entry-content`
   - `body`
4. For each candidate element, compute:
   - visible text chars
   - link text chars
   - link density = link text chars / visible text chars
   - heading count
   - paragraph count
   - code block count
5. Candidate score:
   - `text_chars / 1000.0`
   - plus `paragraph_count * 0.25`
   - plus `heading_count * 0.20`
   - plus `code_block_count * 0.20`
   - minus `link_density * 5.0`
6. Pick the highest-scoring candidate above minimum quality.
7. Fallback to `body` if all candidates are poor.

Minimum quality:

- At least `400` visible chars, unless the whole page is short.
- Link density below `0.55`, unless no better candidate exists.

For non-HTML:

- If `content-type` starts with `text/`, use cleaned raw text.
- If `application/json`, pretty-print bounded JSON only if valid UTF-8 and small enough.
- For PDF, binary, image, audio, video, and archive content, do not return raw bytes. Return a fetch error saying the content type is unsupported for text extraction.

This first pass does not need PDF extraction.

## Chunking

Do not send full fetched text to the model.

Split extracted document text into chunks:

- Normalize whitespace.
- Preserve code block boundaries when possible.
- Target `max_chunk_chars`.
- Use `150` char overlap between adjacent chunks.
- Never split inside a URL if avoidable.

Score chunks against the query:

- Query token coverage.
- Exact phrase match.
- Title/token proximity if the chunk is near headings.
- Small boost for chunks containing version/date terms when the query contains version/date terms.

Keep:

- Up to `max_chunks_per_document`.
- No more than `max_total_chunk_chars` across all documents.

If a fetched document has no good chunks, include no `fetched` evidence for that result and keep its search snippet only.

## Final Re-Ranking

After fetching and chunking, compute `retrieval_score`:

```text
candidate_score
+ extraction_quality_score
+ best_chunk_score * 2.0
+ min(total_chunk_score, 8.0)
- truncation_penalty
```

Truncation penalty:

- `1.0` if the page was truncated.
- `2.0` if it was truncated and produced fewer than 2 chunks.

Sort by final retrieval score. Return the top `returned_results`.

## Tool Output Shape

Change `EnrichedSearchResponse` to return compact evidence:

```json
{
  "query": "bevy 0.17 schedule docs",
  "provider": "searxng",
  "results": [
    {
      "title": "Schedules - Bevy",
      "url": "https://bevy.org/learn/quick-start/getting-started/ecs/#schedules",
      "snippet": "Bevy apps organize systems into schedules that run at specific points in the app lifecycle.",
      "source": "brave",
      "published_at": null,
      "retrieval_score": 14.2,
      "fetched": {
        "final_url": "https://bevy.org/learn/quick-start/getting-started/ecs/#schedules",
        "chunks": [
          {
            "text": "A schedule is a collection of systems to run in a fixed order. Bevy's main app schedule runs every frame, while startup schedules run once when the app starts.",
            "score": 5.8,
            "start_char": 1200,
            "end_char": 2050
          }
        ],
        "text_chars": 18422,
        "bytes": 16384,
        "truncated": true,
        "extraction": {
          "method": "main",
          "text_chars": 18422,
          "link_density": 0.12,
          "quality_score": 9.4
        }
      }
    }
  ],
  "omitted": [
    {
      "title": "Low scoring result",
      "url": "https://example.com/bevy-schedule-overview",
      "reason": "ranked below returned result limit"
    }
  ]
}
```

The model should see enough omitted metadata to know that filtering happened, but not enough to waste context.

## Metadata Sources

Change chat metadata to record only source URLs returned in the final compact evidence pack, plus any explicit `fetch_url` result.

Do not record every raw search candidate as a cited source. That makes the UI imply grounding that may not exist.

Implementation location:

- Update `ChatToolRuntime::invoke_web_search`.
- Build metadata sources from the final ranked documents, not from `response.results` before enrichment.

## Tests

Add tests in `crates/tools/web/src/lib.rs` or module-specific test files.

Required unit tests:

1. `canonicalizes_urls_for_deduplication`
   - Drops fragments and common tracking params.
   - Preserves meaningful query params.

2. `scores_official_docs_above_generic_blog_for_technical_query`
   - Same query, two candidates.
   - Official docs/source host wins.

3. `penalizes_low_signal_titles`
   - "Just a moment" and "Access denied" lose heavily.

4. `extracts_article_before_body`
   - Fixture with nav/sidebar/body/article.
   - Extracted text contains article content and excludes obvious nav text.

5. `falls_back_to_body_for_simple_html`
   - Fixture with only body and paragraphs.

6. `chunks_text_with_budget`
   - Long text produces bounded chunks.
   - Total chunk chars does not exceed configured limit.

7. `chunk_scoring_prefers_query_relevant_text`
   - Relevant chunk ranks above unrelated chunk.

8. `safe_redirect_is_followed`
   - Public URL redirecting to public URL succeeds.

9. `private_redirect_is_blocked`
   - Public URL redirecting to `http://127.0.0.1` fails.

10. `unsupported_binary_content_type_is_not_returned_as_text`
    - PDF/image content type returns a structured unsupported-content error.

Integration tests can use a local HTTP test server if one already exists in the repo. If not, add a tiny `tokio` test server only in test code.

## Rollout

1. Add data model and extraction helpers behind internal functions.
2. Add tests for scoring, extraction, chunking, and redirects.
3. Change `enrich_search_response` to use the new pipeline.
4. Keep the old env vars working.
5. Run the same web-search prompts before and after:
   - "What changed in Bevy's latest ECS scheduling docs?"
   - "Compare current Rust async runtime options for a small service."
   - "Find official docs for llama.cpp OpenAI-compatible tool calling."
6. Inspect tool-result JSON sizes and answer citations.

## Tradeoffs

- More local processing means slightly more code and more tests.
- Bounded parallel fetches improve latency but can hit more remote pages at once.
- Article extraction heuristics are never perfect, but they are much better than raw body text.
- Chunking may omit useful context if query terms are poor. Keeping the original snippet alongside chunks reduces that risk.
- Manual redirect support improves fetch success but must keep strict private-network validation on every hop.

## Definition of Done

- Search tool output is compact and evidence-oriented.
- Fetching first N raw results is replaced by candidate ranking before fetch.
- HTML extraction avoids obvious nav/sidebar/footer noise on fixtures.
- Tool result size is bounded by chunk budgets.
- Chat metadata lists only returned source documents.
- Private-network protection still passes existing and new tests.
