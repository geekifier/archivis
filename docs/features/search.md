# Search

Use the search box on the Library page to browse, filter, sort, and search your collection.

## Search Syntax

### Basic Search

- Type one or more words to search across titles, subtitles, authors, series, publishers, tags, and descriptions.
- Put text in double quotes to search for an exact phrase.

Examples:

```text
dune messiah
"frank herbert"
```

### DSL Search

Use `field:value` to narrow results:

- `author:asimov`
- `series:dune`
- `publisher:ace` or `pub:ace`
- `tag:sci-fi`
- `title:dune`
- `description:spice` or `desc:spice`
- `format:epub` or `fmt:epub`
- `status:identified`
- `resolution:done`
- `outcome:confirmed`
- `trusted:true` or `locked:false`
- `language:en` or `lang:en`
- `year:1965`, `year:1965..1970`, `year:>=1965`
- `has:cover`, `missing:description`, `has:identifiers`
- `isbn:9780441172719`, `asin:B000SEIK2S`, `olid:OL123M`, `hardcover_id:12345`

### Logic And Operators

- Separate terms with spaces to narrow results.
- Use double quotes for exact phrases.
- Prefix a term or clause with `-` to exclude it, for example `-tag:fanfic` or `-"young adult"`.
- Use `OR` between plain terms or quoted phrases, for example `dune OR foundation`.
- Combine clauses as needed, for example `author:"Frank Herbert" format:epub -tag:fanfic`.

`trusted:` and `locked:` accept `true`, `false`, `yes`, `no`, `1`, or `0`.
