# Task Semantics

Use these rules when creating or editing tasks in Djot notes that follow
`djot-tools` semantics.

## Task Shape

A task is a Djot div whose class is exactly `task`:

```djot
::: task
Write the parser.

Optional details.
:::
```

The task title is the plain text of the first paragraph inside the task div.
Later blocks are details.

Metadata can be attached to the task div:

```djot
{#write-parser created="2026-06-18T09:00:00+08:00"}
::: task
Write the parser.
:::
```

Metadata can also be attached to the list item that directly contains the task:

```djot
- {created="2026-06-18T09:00:00+08:00"}
  ::: task
  Write the parser.
  :::
```

Task div metadata takes precedence over inherited list item metadata.

## Fields

Only these task metadata fields have defined semantics:

- `id` / `{#id}`: task anchor id.
- `created`: quoted RFC 3339 datetime for creation time.
- `due`: quoted RFC 3339 datetime for the current instance deadline.
- `wait`: quoted RFC 3339 datetime for the earliest actionable time.
- `done`: quoted RFC 3339 datetime for completion time.
- `canceled`: quoted RFC 3339 datetime for cancellation time.
- `recur`: supported ISO 8601 date duration: `PnD`, `PnW`, `PnM`, or `PnY`.
- `prev`: previous recurring instance reference.
- `depends`: whitespace-separated dependency references.

Do not use date-only values for datetime fields. Use quoted RFC 3339 datetimes,
for example `2026-06-18T09:00:00+08:00` or `2026-06-18T01:00:00Z`.

Do not create undefined task fields such as `status`, `scheduled`, `priority`,
or `tags` unless the user explicitly wants ordinary custom Djot attributes with
no `djot-tools` task semantics.

Native Djot task list items such as `- [ ] Write parser` and `- [x] Done` are
not the current semantic task model. Convert them to `::: task` divs before
using semantic task tooling.

## References

Task reference attributes use Djot link-destination spelling:

- Same file: `#task-id`
- Cross file: `path.dj#task-id`

Use this spelling for both `prev` and `depends`. Do not use bare ids such as
`depends="draft"` or `prev="weekly-review"`.

If a cross-file path contains spaces or other bytes that cannot safely appear in
a whitespace-separated attribute value, percent-encode the path:

```djot
{depends="Project%20Plan.dj#review"}
::: task
Publish.
:::
```

## Completing And Canceling

Complete an open task by adding a `done` timestamp. Cancel an open task by
adding a `canceled` timestamp. Do not set both fields on the same task.

```djot
{#write-parser done="2026-06-19T21:30:00+08:00"}
::: task
Write the parser.
:::
```

## Recurring Tasks

A task with both `due` and `recur` is a recurring task instance. When completing
one:

1. Keep the completed instance in place.
2. Add `done` to the completed instance.
3. Append a new open task instance.
4. Advance `due` by the repeat rule.
5. If `wait` exists, advance it by the same repeat rule.
6. Keep `recur`.
7. Set `prev` on the new instance to the completed task id.

Example:

```djot
{#weekly-review due="2026-06-21T17:00:00+08:00" recur="P1W" done="2026-06-21T18:00:00+08:00"}
::: task
Weekly review.
:::

{#Weekly-review-2026-06-28 created="2026-06-21T18:00:00+08:00" due="2026-06-28T17:00:00+08:00" recur="P1W" prev="#weekly-review"}
::: task
Weekly review.
:::
```

For list-shaped recurring tasks where the task div is the only substantive
content of the list item, append the next instance as a new sibling list item:

```djot
- {#daily-review due="2026-06-21T17:00:00+08:00" recur="P1D" done="2026-06-21T18:00:00+08:00"}
  ::: task
  Daily review.
  :::

- {#Daily-review-2026-06-22 created="2026-06-21T18:00:00+08:00" due="2026-06-22T17:00:00+08:00" recur="P1D" prev="#daily-review"}
  ::: task
  Daily review.
  :::
```

If the original list item contains additional nonblank content after the task,
keep that content with the original item and avoid moving it.

## Dependencies

Use `depends` for blocking relationships. Dependencies are actionable when every
referenced task is done or canceled.

```djot
{#publish depends="#draft review.dj#approve"}
::: task
Publish.
:::
```

Avoid self-dependencies and dependency cycles.
