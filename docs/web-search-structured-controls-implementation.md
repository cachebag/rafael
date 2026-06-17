# Rafael Web Search Structured Controls Implementation

## Goal

Expose structured search controls to the model and map them to real provider capabilities. This lets Rafael ask for recent results, official domains, language/country targeting, and provider-specific categories without relying only on brittle query text.

This implements suggestion 4 from the quality review: add structured controls such as recency, domains, exclusions, source preference, and search kind.

## Current State

The model-facing `web_search` tool accepts only:

```json
{
  "query": "string",
  "max_results": 5
}
```

The provider layer supports:

- SearXNG with hardcoded `categories=general`, `safesearch=1`, and `pageno=1`.
- Brave with hardcoded `extra_snippets=true` and `count=max_results`.

Official provider capabilities that should be used:

- SearXNG Search API supports `q`, `categories`, `engines`, `language`, `pageno`, `time_range`, `format`, and `safesearch`.
- Brave Web Search supports `freshness`, `country`, `search_lang`, `safesearch`, `count`, `offset`, extra snippets, and search operators inside `q`.
- Brave also offers an LLM Context endpoint optimized for LLM grounding. That can be a later provider mode, but this document focuses on structured controls for the existing `web_search` path.

## Design Principles

- Keep the model-facing schema flat where possible. Local models are more reliable with simple fields than deeply nested objects.
- Validate all structured fields before provider calls.
- Map unsupported provider features gracefully.
- Keep query text human-readable. Do not hide too much behavior in provider-specific magic.
- Never let domain filters bypass URL safety validation for fetched pages.

## Model-Facing Tool Schema

Replace `WebSearchArgs` with:

```rust
#[derive(Debug, Deserialize)]
struct WebSearchArgs {
    query: String,
    max_results: Option<usize>,

    #[serde(default)]
    recency: Option<SearchRecencyArg>,
    #[serde(default)]
    freshness_start: Option<String>,
    #[serde(default)]
    freshness_end: Option<String>,

    #[serde(default)]
    include_domains: Vec<String>,
    #[serde(default)]
    exclude_domains: Vec<String>,

    #[serde(default)]
    source_preference: Option<SourcePreferenceArg>,
    #[serde(default)]
    search_kind: Option<SearchKindArg>,

    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    country: Option<String>,
    #[serde(default)]
    safesearch: Option<SafeSearchArg>,

    #[serde(default)]
    page: Option<usize>,
    #[serde(default)]
    engines: Vec<String>,
    #[serde(default)]
    categories: Vec<String>,
}
```

Enums:

```rust
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SearchRecencyArg {
    Day,
    Week,
    Month,
    Year,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SourcePreferenceArg {
    Official,
    Documentation,
    SourceCode,
    Primary,
    Broad,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SearchKindArg {
    Web,
    News,
    Docs,
    Code,
    Papers,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SafeSearchArg {
    Off,
    Moderate,
    Strict,
}
```

Tool JSON schema:

```json
{
  "type": "object",
  "properties": {
    "query": {
      "type": "string",
      "description": "Search query. Keep it concise and include core entities, versions, or error messages."
    },
    "max_results": {
      "type": "integer",
      "minimum": 1,
      "maximum": 8,
      "description": "Maximum source documents to return."
    },
    "recency": {
      "type": "string",
      "enum": ["day", "week", "month", "year"],
      "description": "Optional freshness filter. Use for current or recently changed facts."
    },
    "freshness_start": {
      "type": "string",
      "description": "Optional YYYY-MM-DD start date. Use with freshness_end for an exact date range."
    },
    "freshness_end": {
      "type": "string",
      "description": "Optional YYYY-MM-DD end date. Use with freshness_start for an exact date range."
    },
    "include_domains": {
      "type": "array",
      "items": { "type": "string" },
      "description": "Optional domains that results should come from, such as docs.rs or bevy.org."
    },
    "exclude_domains": {
      "type": "array",
      "items": { "type": "string" },
      "description": "Optional domains to exclude."
    },
    "source_preference": {
      "type": "string",
      "enum": ["official", "documentation", "source_code", "primary", "broad"],
      "description": "Optional preference used to rewrite and rerank results."
    },
    "search_kind": {
      "type": "string",
      "enum": ["web", "news", "docs", "code", "papers"],
      "description": "Optional search intent. Use docs/code/papers for technical research when appropriate."
    },
    "language": {
      "type": "string",
      "description": "Optional language code, such as en, en-US, de, or fr."
    },
    "country": {
      "type": "string",
      "description": "Optional 2-letter country code for providers that support regional results."
    },
    "safesearch": {
      "type": "string",
      "enum": ["off", "moderate", "strict"],
      "description": "Optional safe search level."
    },
    "page": {
      "type": "integer",
      "minimum": 1,
      "maximum": 10,
      "description": "Optional result page. Use only when earlier results were insufficient."
    },
    "engines": {
      "type": "array",
      "items": { "type": "string" },
      "description": "Optional SearXNG engine names. Ignored by providers that do not support engine selection."
    },
    "categories": {
      "type": "array",
      "items": { "type": "string" },
      "description": "Optional SearXNG categories, such as general, news, it, science, files, images, or videos."
    }
  },
  "required": ["query"],
  "additionalProperties": false
}
```

## Internal Search Options

Add provider-independent options to the `web` crate.

```rust
#[derive(Debug, Clone)]
pub struct SearchOptions {
    pub max_results: usize,
    pub page: usize,
    pub freshness: Option<SearchFreshness>,
    pub include_domains: Vec<DomainFilter>,
    pub exclude_domains: Vec<DomainFilter>,
    pub source_preference: SourcePreference,
    pub search_kind: SearchKind,
    pub language: Option<String>,
    pub country: Option<String>,
    pub safesearch: SafeSearch,
    pub engines: Vec<String>,
    pub categories: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum SearchFreshness {
    Recency(SearchRecency),
    DateRange { start: chrono::NaiveDate, end: chrono::NaiveDate },
}

#[derive(Debug, Clone, Copy)]
pub enum SearchRecency {
    Day,
    Week,
    Month,
    Year,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomainFilter(String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourcePreference {
    Official,
    Documentation,
    SourceCode,
    Primary,
    Broad,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchKind {
    Web,
    News,
    Docs,
    Code,
    Papers,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SafeSearch {
    Off,
    Moderate,
    Strict,
}
```

Default options:

- `page = 1`
- `freshness = None`
- `source_preference = Broad`
- `search_kind = Web`
- `safesearch = Moderate`
- `categories = ["general"]` for SearXNG if absent

Add `chrono` to `crates/tools/web/Cargo.toml` for date validation.

## Validation

Add `WebError` variants:

```rust
InvalidDomainFilter(String)
InvalidLanguage(String)
InvalidCountry(String)
InvalidDate(String)
InvalidDateRange { start: String, end: String }
UnsupportedSearchOption { provider: String, option: String }
```

Validation rules:

- `query` must be non-empty after trimming.
- `max_results` is clamped by chat config as it is today.
- `page` clamps to `1..=10`.
- `language` must match `^[A-Za-z]{2,3}(-[A-Za-z]{2})?$`.
- `country` must match `^[A-Za-z]{2}$` and is uppercased.
- Domain filters:
  - Must parse as a host or URL with host.
  - Strip scheme and path if a URL is supplied.
  - Lowercase.
  - Reject `localhost`, `.local`, bare single-label hosts, IP literals, private IPs, and wildcard-only domains.
  - Allow leading `www.` but normalize it away for comparisons.
- Date range:
  - Both `freshness_start` and `freshness_end` must be present together.
  - Must parse as `YYYY-MM-DD`.
  - `start <= end`.
  - If date range is present, it takes precedence over `recency`.

Domain filters are for search ranking/querying only. Fetched result URLs still go through `validate_public_url`.

## Query Rewriting

Implement a provider-independent `SearchQueryPlan`.

```rust
struct SearchQueryPlan {
    base_query: String,
    provider_query: String,
    include_domains: Vec<DomainFilter>,
    exclude_domains: Vec<DomainFilter>,
    notes: Vec<String>,
}
```

Build it from `query`, `source_preference`, `search_kind`, and domain filters.

### Domain Filters

Provider search APIs generally support domain filtering through query operators, not dedicated fields.

Rules:

- For one include domain, append `site:domain` to the query.
- For multiple include domains, run one provider query per included domain and merge/de-duplicate results.
- For each exclude domain, append `-site:domain`.
- Never put user-provided domain strings into the query until normalized by `DomainFilter`.

Example:

```text
query: "bevy schedule states"
include_domains: ["bevy.org", "docs.rs"]
```

Run:

```text
bevy schedule states site:bevy.org
bevy schedule states site:docs.rs
```

Then merge candidates by canonical URL.

### Source Preference

`source_preference` changes query terms and ranking hints.

- `official`
  - Add query hint: `official docs` unless already present.
  - Boost official domains during quality scoring.
- `documentation`
  - Add query hint: `documentation docs`.
  - Boost docs-like hosts.
- `source_code`
  - Add query hint: `github source code` unless an include domain already targets a source host.
  - Boost `github.com`, `gitlab.com`, and project repo hosts.
- `primary`
  - Add no broad text hint.
  - Boost official docs, source repos, standards bodies, release notes, and vendor docs.
- `broad`
  - No rewrite.

Do not make these rewrites too aggressive. They are hints, not hard filters.

### Search Kind

`search_kind` maps to provider options and query hints:

- `web`
  - Default.
- `news`
  - SearXNG: use category `news` if no explicit categories were supplied.
  - Brave Web Search: keep web endpoint for the first pass; add query hint only if needed.
- `docs`
  - Add source preference `documentation` if caller did not specify one.
  - SearXNG: category `it` can be added for technical docs only if the query looks technical.
- `code`
  - Add source preference `source_code` if caller did not specify one.
  - Add query hint `github`.
- `papers`
  - Add query hint `paper OR arxiv OR doi` only if the provider handles operators well.
  - SearXNG: use `science` category if available or configured.

Provider-specific category names vary by SearXNG instance. If an explicit category causes a provider error, surface the error rather than silently changing user intent.

## Provider Mapping

### Brave Web Search

Endpoint remains:

```text
GET https://api.search.brave.com/res/v1/web/search
```

Map options:

- `q`: `SearchQueryPlan.provider_query`
- `count`: `max_results.min(20)`
- `offset`: `page - 1`
- `extra_snippets`: `true`
- `freshness`:
  - day -> `pd`
  - week -> `pw`
  - month -> `pm`
  - year -> `py`
  - date range -> `YYYY-MM-DDtoYYYY-MM-DD`
- `country`: uppercase 2-letter country.
- `search_lang`: language lowercased where appropriate.
- `safesearch`:
  - off -> `off`
  - moderate -> `moderate`
  - strict -> `strict`

Ignored options:

- `engines`
- `categories`

If ignored options are present, include a `provider_notes` field in the search response:

```json
["Brave Web Search does not support engine/category selection; ignored engines/categories."]
```

### SearXNG

Endpoint remains:

```text
GET {base_url}/search
```

Map options:

- `q`: `SearchQueryPlan.provider_query`
- `format`: `json`
- `categories`: comma-separated categories, default `general`
- `engines`: comma-separated engines when provided
- `language`: language code
- `pageno`: page
- `time_range`:
  - day -> `day`
  - week -> `week`
  - month -> `month`
  - year -> `year`
  - exact date range -> unsupported, fall back to closest recency only if one can be inferred; otherwise add a provider note.
- `safesearch`:
  - off -> `0`
  - moderate -> `1`
  - strict -> `2`

Ignored options:

- `country`

If `country` is present, add a provider note. Do not fake country targeting unless a specific SearXNG instance is configured to support regional engine params.

## Multiple Domain Queries

When `include_domains.len() > 1`, execute one provider search per include domain.

Algorithm:

1. Build one query plan per include domain.
2. Run provider searches concurrently with limit `3`.
3. Merge results.
4. De-duplicate by canonical URL.
5. Preserve the best provider rank per URL.
6. Re-rank with the quality pipeline.
7. Return top `max_results`.

If one domain-specific query fails and others succeed:

- Return successful results.
- Add a provider note for failed domain query.

If all fail:

- Return a tool error.

## Response Shape

Extend `SearchResponse`:

```rust
pub struct SearchResponse {
    pub query: String,
    pub provider: String,
    pub results: Vec<SearchResult>,
    pub options: AppliedSearchOptions,
    pub provider_notes: Vec<String>,
}

pub struct AppliedSearchOptions {
    pub freshness: Option<String>,
    pub include_domains: Vec<String>,
    pub exclude_domains: Vec<String>,
    pub source_preference: String,
    pub search_kind: String,
    pub language: Option<String>,
    pub country: Option<String>,
    pub safesearch: String,
    pub page: usize,
    pub engines: Vec<String>,
    pub categories: Vec<String>,
}
```

The model should see `provider_notes`. Notes help it avoid assuming a filter was applied when it was ignored.

## Tool Prompt Update

Update `WEB_TOOL_SYSTEM_PROMPT` to teach the model when to use structured fields:

```text
Use web_search for current, version-sensitive, or source-grounded information. Use recency for recent facts, include_domains for official/project docs, exclude_domains to avoid low-quality repeats, source_preference for official/docs/source-code/primary-source intent, and search_kind for docs/code/news/papers. Prefer fetched evidence chunks over snippets. Cite source URLs used in the final answer.
```

Keep this prompt short. The schema descriptions should carry most of the detail.

## Tests

Add provider-independent tests in `crates/tools/web`.

Required tests:

1. `validates_domain_filters`
   - Accepts `https://docs.rs/foo`, normalizes to `docs.rs`.
   - Rejects `localhost`, `printer.local`, `192.168.1.1`, and `intranet`.

2. `validates_language_and_country`
   - Accepts `en`, `en-US`, `de`.
   - Accepts country `us` and normalizes to `US`.
   - Rejects malformed values.

3. `date_range_takes_precedence_over_recency`
   - Both date range and recency supplied.
   - Applied options use date range.

4. `brave_maps_recency_to_freshness`
   - day/week/month/year map to `pd/pw/pm/py`.

5. `brave_maps_date_range_to_freshness`
   - `2026-01-01` plus `2026-01-31` maps to `2026-01-01to2026-01-31`.

6. `brave_maps_language_country_and_safesearch`
   - URL query contains `search_lang`, `country`, and `safesearch`.

7. `searxng_maps_categories_engines_language_page_time_range`
   - URL query contains expected comma-separated values.

8. `searxng_notes_unsupported_country`
   - Country option creates provider note.

9. `multiple_include_domains_create_multiple_queries`
   - Query plans include exactly one `site:` operator each.

10. `exclude_domains_are_added_to_provider_query`
    - Query contains normalized `-site:` operators.

11. `source_preference_rewrites_query_lightly`
    - Documentation preference adds docs hint.
    - Broad preference leaves query unchanged.

12. `tool_schema_rejects_unknown_properties`
    - `additionalProperties: false` remains present.

Where URL-building is currently embedded inside async provider calls, extract pure helpers:

```rust
fn build_brave_search_url(query: &str, options: &SearchOptions) -> Result<Url, WebError>
fn build_searxng_search_url(base_url: &str, query: &str, options: &SearchOptions) -> Result<Url, WebError>
```

This makes provider mapping testable without network calls.

## Rollout

1. Add internal `SearchOptions` fields and validation.
2. Add URL-building helper tests.
3. Expand `WebSearchArgs` and the OpenAI tool schema.
4. Map options to Brave and SearXNG.
5. Add provider notes to `SearchResponse`.
6. Update `WEB_TOOL_SYSTEM_PROMPT`.
7. Update `services/chat/README.md` with examples:

```json
{"query":"Bevy 0.17 schedules","include_domains":["bevy.org"],"source_preference":"documentation"}
```

```json
{"query":"llama.cpp tool calling OpenAI compatible","recency":"month","include_domains":["github.com","github.io"],"source_preference":"primary"}
```

8. Test with local SearXNG and Brave if a Brave key is configured.

## Tradeoffs

- A richer schema helps good tool use, but local models may sometimes fill fields incorrectly. Strict validation and clear tool errors are required.
- Domain include filters can reduce recall. The model should use them for official-doc searches, not for broad discovery.
- Multiple include domains increase provider calls. Limit concurrency and keep result caps low.
- SearXNG capabilities vary by configured engines. Provider notes are necessary because some filters are best-effort.
- Brave LLM Context may outperform custom fetch/chunking, but it is a separate product path with different cost/privacy tradeoffs.

## Definition of Done

- `web_search` accepts structured recency, domain, language, country, page, safe search, source preference, and search kind fields.
- Brave receives real `freshness`, `country`, `search_lang`, `safesearch`, `count`, and `offset` params.
- SearXNG receives real `categories`, `engines`, `language`, `pageno`, `time_range`, and `safesearch` params.
- Unsupported provider options produce visible provider notes.
- Domain filters are normalized and validated.
- Multiple include domains work by query fan-out and de-duplication.
- Unit tests cover validation and provider URL mapping.
