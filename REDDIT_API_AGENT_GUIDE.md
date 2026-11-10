# Neddit API guide for Reddit-search agents

## Service

- Base URL: `https://n.r2rn.org`
- JSON API prefix: `https://n.r2rn.org/api`
- Authentication: none
- Access: read-only
- Health check: `GET /healthz` returns `204 No Content`
- Do not add Reddit-style `.json` suffixes.
- URL-encode every query parameter. With `curl`, prefer `-G --data-urlencode`.

The service depends on Reddit upstream. Treat `429`, `502`, `504`, `520`, and `522` as transient unless the response says otherwise. Retry idempotent GET requests with exponential backoff and a reasonable request timeout.

## Recommended search endpoint: QL search

Use this endpoint for agent-driven search:

```text
GET https://n.r2rn.org/api/search/ql
```

It discovers candidates through Reddit, then applies deterministic local filters. It supports posts, comments, and communities and returns JSON.

### Request parameters

| Parameter | Required | Values | Notes |
|---|---:|---|---|
| `q` | yes | QL expression | Maximum 4096 UTF-8 bytes and 256 expression nodes. |
| `kind` | no | `posts`, `comments`, `communities` | Defaults to `posts`. |
| `sort` | no | Mode-specific values below | Defaults to `relevance`. |
| `t` | no | `hour`, `day`, `week`, `month`, `year`, `all` | Valid only when `sort=top`; defaults to `all`. |
| `limit` | no | `25`, `50`, `100` | Defaults to `25`. |
| `include_nsfw` | no | `true`, `false` | Defaults to `false`. |
| `cursor` | no | Cursor from a prior response | Repeat every other request parameter unchanged. |

Supported sorts:

| Kind | Sorts |
|---|---|
| `posts` | `relevance`, `new`, `top`, `hot` |
| `comments` | `new`, `top` |
| `communities` | `relevance`, `activity` |

### QL operators

| Syntax | Meaning | Example |
|---|---|---|
| `word` | Match one normalized whole word | `kernel` |
| `"words together"` | Match words in sequence | `"borrow checker"` |
| space, `AND`, `&&` | Require both expressions | `rust async` |
| `OR`, `||` | Accept either expression | `rust OR programming` |
| `NOT`, `!` | Exclude an expression | `!flair:Help` |
| `(...)` | Group expressions | `(rust OR programming) async` |
| `field:value` | Search one field | `author:alice` |
| `field:(...)` | Apply one field to a group | `subreddit:(rust OR programming)` |
| `\"`, `\\` | Escape quotes or backslashes inside quotes | `title:"say \"hello\""` |

The textual operators `AND`, `OR`, and `NOT` are uppercase. Plain terms are case-insensitive and match whole normalized words.

### Post fields

| Field | Meaning | Example |
|---|---|---|
| no prefix | Title or self-text | `"async rust"` |
| `subreddit:` | Exact subreddit name | `subreddit:rust` |
| `author:` | Exact author | `author:alice` |
| `title:` | Post title | `title:"linux kernel"` |
| `domain:` | Exact link domain | `domain:example.com` |
| `flair:` | Post flair text | `flair:News` |
| `past:` | Age using `m`, `h`, `d`, or `w` | `past:7d` |
| `after:` | On or after a date or RFC 3339 instant | `after:2025-01-01` |
| `before:` | Before a date or RFC 3339 instant | `before:2026-01-01` |

A post search needs a positive discovery scope such as text, subreddit, author, title, domain, or flair. A query made entirely from exclusions or dates cannot discover candidates.

### Comment fields

| Field | Meaning | Example |
|---|---|---|
| no prefix | Comment body | `"borrow checker"` |
| `subreddit:` | Exact subreddit | `subreddit:rust` |
| `author:` | Exact comment author | `author:alice` |
| `post.title:` | Parent post title | `post.title:release` |
| `post.domain:` | Parent post domain | `post.domain:example.com` |
| `post.flair:` | Parent post flair | `post.flair:News` |
| `past:` | Comment age | `past:24h` |
| `after:` | On or after a date or instant | `after:2025-01-01` |
| `before:` | Before a date or instant | `before:2026-01-01` |

Comment discovery requires one shared positive `subreddit:` or `author:` scope. Global historical comment search is unavailable. Comment results cover only the available recent subreddit or author history.

### Community fields

| Field | Meaning | Example |
|---|---|---|
| no prefix | Name, title, or description | `programming` |
| `name:` | Exact subreddit name | `name:rust` |
| `title:` | Community title | `title:programming` |
| `description:` | Public description | `description:"systems programming"` |

Exact names can be combined, for example `name:(rust OR programming)`. Exact-name lookup supports at most 100 names and only `sort=relevance`.

### QL examples

Search recent Rust posts about the borrow checker:

```bash
curl -fsSG 'https://n.r2rn.org/api/search/ql' \
  --data-urlencode 'q=subreddit:rust "borrow checker" past:30d' \
  --data 'kind=posts' \
  --data 'sort=relevance' \
  --data 'limit=25'
```

Search recent comments within one subreddit:

```bash
curl -fsSG 'https://n.r2rn.org/api/search/ql' \
  --data-urlencode 'q=subreddit:rust "borrow checker" past:7d' \
  --data 'kind=comments' \
  --data 'sort=new' \
  --data 'limit=25'
```

Find communities related to systems programming:

```bash
curl -fsSG 'https://n.r2rn.org/api/search/ql' \
  --data-urlencode 'q=description:"systems programming"' \
  --data 'kind=communities' \
  --data 'sort=relevance' \
  --data 'limit=25'
```

### QL response

```json
{
  "items": [
    {
      "kind": "t3",
      "data": {
        "title": "...",
        "subreddit": "rust",
        "author": "...",
        "permalink": "/r/rust/comments/..."
      }
    }
  ],
  "number": 1,
  "previous_cursor": null,
  "cursor": "uuid.1",
  "ranking": "reddit.relevance",
  "continuation": "buffered_results_remain",
  "coverage": ["..."],
  "observed_matches": 47,
  "frozen_at": "2026-09-25T15:14:04Z",
  "text_profile": "nfc-caseless-0.2.2-nfc-0.1.25-ql1"
}
```

Common Reddit object kinds:

- `t1`: comment
- `t3`: post
- `t5`: subreddit/community

`observed_matches` is not a global result count. Search is bounded and can stop before Reddit exhausts all possible candidates. Read `coverage` and `continuation` before claiming completeness.

Possible `continuation` values:

- `buffered_results_remain`
- `more_results_may_exist`
- `selected_sources_exhausted`
- `stopped_early`

### QL pagination

Use the returned `cursor` and repeat the original controls exactly:

```bash
curl -fsSG 'https://n.r2rn.org/api/search/ql' \
  --data-urlencode 'q=subreddit:rust "borrow checker" past:30d' \
  --data 'kind=posts' \
  --data 'sort=relevance' \
  --data 'limit=25' \
  --data-urlencode 'cursor=CURSOR_FROM_PREVIOUS_RESPONSE'
```

Cursors expire after 15 minutes. Each retained search session supports up to four pages. A malformed, expired, evicted, or mismatched cursor returns HTTP `410` with `code: "invalid_cursor"`.

### QL errors

QL errors use this shape:

```json
{
  "code": "scope_required",
  "message": "Add a positive text, community, author, title, or domain scope",
  "start": 0,
  "end": 12
}
```

`start` and `end` are byte offsets into `q`.

Useful status codes:

- `400`: invalid syntax, field, sort, mode, or controls
- `410`: expired or invalid cursor
- `429`: upstream or service rate limit
- `502`: Reddit/source failure
- `503`: search concurrency or storage limit
- `504`: 20-second search deadline exceeded

The QL search concurrency limit is four active source searches.

## Reddit-compatible search

Neddit also exposes a thinner Reddit-compatible search endpoint:

```text
GET /api/search
GET /api/r/{subreddit}/search
```

Important parameters:

| Parameter | Values |
|---|---|
| `q` | Reddit search expression, maximum 512 characters |
| `type` | Comma-separated `link`, `sr`, or `user` |
| `sort` | `relevance`, `hot`, `top`, `new`, `comments` |
| `t` | `hour`, `day`, `week`, `month`, `year`, `all` |
| `restrict_sr` | `true` or `false` |
| `include_over_18` | `true` or `false` |
| `after`, `before` | Reddit listing cursor |

Example:

```bash
curl -fsSG 'https://n.r2rn.org/api/search' \
  --data-urlencode 'q="rust async"' \
  --data 'type=link' \
  --data 'sort=relevance' \
  --data 'include_over_18=false'
```

Prefer `/api/search/ql` for new integrations. The deployed Reddit-compatible search currently rejects a supplied numeric `limit` because of its flattened query decoder. Omit `limit` on this route; QL search supports page sizes correctly.

The Reddit-compatible response uses the standard listing shape:

```json
{
  "kind": "Listing",
  "data": {
    "after": "t3_...",
    "before": null,
    "children": [
      {
        "kind": "t3",
        "data": {
          "id": "...",
          "name": "t3_...",
          "title": "...",
          "author": "...",
          "subreddit": "...",
          "permalink": "/r/.../comments/...",
          "url": "...",
          "selftext": "...",
          "score": 123,
          "num_comments": 45,
          "created_utc": 1700000000.0,
          "over_18": false
        }
      }
    ]
  }
}
```

## Other useful JSON routes

All paths below are relative to `https://n.r2rn.org/api`.

### Listings

```text
GET /
GET /hot
GET /new
GET /rising
GET /top?t=day
GET /controversial?t=week
GET /r/{subreddit}
GET /r/{subreddit}/hot
GET /r/{subreddit}/new
GET /r/{subreddit}/rising
GET /r/{subreddit}/top?t=month
GET /r/{subreddit}/controversial?t=all
```

Common listing parameters are `after`, `before`, `limit` from 1 through 100, `count`, `show=all`, `t`, and `sr_detail`.

The web homepage uses Reddit's non-localized `/r/all` feed. The JSON front-page routes remain Reddit-compatible and may reflect Reddit's upstream regional behavior.

### Posts and comments

```text
GET /comments/{post_id}
GET /r/{subreddit}/comments/{post_id}
GET /comments/{post_id}/{slug}
GET /r/{subreddit}/comments/{post_id}/{slug}
GET /by_id/{comma-separated-fullnames}
GET /duplicates/{post_id}
```

Useful comment parameters include `sort=confidence|top|new|controversial|old`, `limit`, `depth`, `context`, and `comment={comment_id}`.

### Subreddits

```text
GET /r/{subreddit}/about
GET /r/{subreddit}/sidebar
GET /subreddits/popular
GET /subreddits/new
GET /subreddits/default
GET /subreddits/search?q=QUERY
```

### Users

```text
GET /user/{username}/about
GET /user/{username}
GET /user/{username}/overview
GET /user/{username}/submitted
GET /user/{username}/comments
GET /v1/user/{username}/trophies
GET /users/new
GET /users/popular
GET /users/search?q=QUERY
```

### Lookup

```text
GET /info?id=t3_POST_ID,t1_COMMENT_ID
GET /r/{subreddit}/api/info?id=t3_POST_ID
```

## Agent extraction examples

Extract compact post records from a QL response:

```bash
curl -fsSG 'https://n.r2rn.org/api/search/ql' \
  --data-urlencode 'q=subreddit:rust async' \
  --data 'kind=posts' \
  --data 'sort=new' \
  --data 'limit=25' |
jq '[.items[] | select(.kind == "t3") | .data | {
  id,
  title,
  author,
  subreddit,
  permalink,
  url,
  selftext,
  score,
  num_comments,
  created_utc,
  over_18
}]'
```

Extract compact comments:

```bash
curl -fsSG 'https://n.r2rn.org/api/search/ql' \
  --data-urlencode 'q=subreddit:rust "borrow checker"' \
  --data 'kind=comments' \
  --data 'sort=new' \
  --data 'limit=25' |
jq '[.items[] | select(.kind == "t1") | .data | {
  id,
  author,
  body,
  subreddit,
  link_id,
  parent_id,
  permalink,
  score,
  created_utc
}]'
```

## Minimal instructions for another agent

```text
Use https://n.r2rn.org/api/search/ql as the primary Reddit search API.
Send GET requests with URL-encoded parameters.
Use kind=posts, comments, or communities.
Use limit=25, 50, or 100.
For comments, include one positive subreddit: or author: scope.
Follow the response cursor with the exact same query controls.
Treat observed_matches as an observed count, not a total.
Read coverage and continuation before claiming exhaustive results.
Retry transient 429/502/504/520/522 failures with exponential backoff.
Fetch a chosen post's discussion from /api/comments/{post_id}.
```
