# 0034. Choosing a model: a fetched catalogue, an override that belongs to the conversation, and a field that stays typeable

## Status

Accepted

## Context

ADR-0021 settled how the AI panel picks a *provider*: four dialects behind one `ProviderKind`, each configured with a base URL, an environment variable naming its key, and a model id.
It said nothing about picking a *model*, because at the time the model was simply another configuration field.

In use that turns out to be two problems.

The first is discovery.
The model id is the one part of a provider's configuration that changes faster than this IDE ships — `providers::default_catalog` says so in a comment — and a free-text field means the user has to know the exact spelling by heart.
A typo is not reported by the settings page; it is reported by the vendor, as a 404, after a message has been composed and sent.

The second is scope.
Because the model lived on the provider row, choosing one was a global, sticky decision.
Asking a cheap fast model one question and a large one the next meant opening Settings between them, which is enough friction that nobody does it, which means every conversation runs on whatever was configured months ago.

## Decision

### 1. The model catalogue is fetched from the provider, not compiled in

`ai_chat_core::models::list_models` asks the configured endpoint what it offers: `GET /v1/models` for Anthropic and both OpenAI dialects, `GET /v1beta/models` for Gemini.
The dialect differences are confined to `catalog_url` and the pure `parse_models`, the same shape `request.rs` and `stream.rs` already have, so a fifth provider remains a match arm and a fixture test.

A hard-coded table was rejected for the reason the free-text field existed in the first place: a list of model ids baked into a release is wrong within weeks, and being wrong quietly — offering a menu of models that no longer exist — is worse than offering nothing.

This is deliberately **discovery**, and it is the one place this project does it.
ADR-0021 states that provider capabilities are *declared, not discovered*, and that stays true: whether a dialect supports tools, images or explicit caching is a property of the code in `request.rs` and cannot be learned from a catalogue.
A model catalogue is the opposite — it is exactly what the vendor publishes and exactly what this IDE cannot know.

### 2. The list is a convenience and never a gate

Both pickers — the panel header and the Settings cell — are **editable** combo boxes, and both show model *ids*, with the provider's friendlier display name as a tooltip.

Three consequences follow, all deliberate:

- A model the catalogue does not list — a preview id, a fine-tune, an alias an OpenAI-compatible gateway invents — is still reachable by typing, which is the behaviour the field has always had.
- A failed fetch costs the user nothing but the menu: `models_status` says what happened in a sentence composed in Rust, and the field still works.
- Because the box is editable, whatever it displays is also what a user can type and what `setModel` is handed. Showing a display name would make the label a second spelling of the choice and would eventually persist "Claude Sonnet 5" as a model id.

An unrecognised response envelope parses to an empty list rather than an error, and a catalogue entry with no id is skipped rather than failing the listing: one unreadable row must not cost the user the other forty.

### 3. The fetch happens when the picker is opened, and never before

Nothing contacts a provider because a panel was constructed or a combo repainted.
`AiChatPanel` uses a small `LazyComboBox` whose `showPopup` asks — Qt publishes no "about to show popup" signal — and the settings page fetches from the delegate's `createEditor`, i.e. when the user opens that cell.

This is a privacy decision more than a performance one.
This is the feature that talks to a third party, and an outbound request the user did not ask for, at application start, to whichever endpoint a settings file happens to name, is not something they consented to by opening an editor.

Like every other blocking call in this feature (ADR-0021 §4), the fetch runs on one `std::thread` and marshals back with `CxxQtThread::queue`; the settings dialog therefore gained a `Threading` impl. Results are cached per provider id, with no TTL — invalidated by a provider switch, by the settings dialog closing, and by `beginEdit`.

### 4. The chosen model belongs to the conversation, not the provider

`Conversation::model: Option<String>` overrides the provider's configured default; `None` means "use the default", which is what a fresh conversation starts as.

The override is applied in exactly one place — `AiChat::provider()`, the single function that builds the `ProviderConfig` a request is made from — so sending, token counting and Gemini's path-embedded model all pick it up with no second application and no way for them to disagree.

Three rules fall out of the scope choice and are tested:

- **A new conversation returns to the provider's default.** `Conversation::clear` drops the override with the transcript; the settings page owns the default, and the picker owns the exception.
- **Switching provider clears the override.** A model id from one vendor means nothing to another, and sending one is a 404 the user did not cause.
- **It persists with the transcript.** The field is `#[serde(default)]` on `Conversation`, which `ConversationRecord` already embeds, so a record written before this change loads as `None` and needs no migration — and loading an old conversation restores the model it ran on.

The rejected alternative was to keep the choice on the provider row and let the header combo edit it.
It is less code, and it silently rewrites a persisted setting every time someone tries a model for one question — the sticky-global problem restated rather than solved.

## Consequences

- The Model column in Settings > AI Providers still stores and validates exactly what it did; only the editor in front of it changed. `settings_model::ai::validate`'s "this provider needs a model name" rule is untouched.
- The panel header no longer repeats the model in the provider entry (`"Anthropic · claude-sonnet-5"` became `"Anthropic"`), because the model now has its own control beside it.
- `transport.rs` gained `get_json` and a shared `credential_header`, so the catalogue fetch reuses the module that holds the key and constructs the redacted errors — there is still exactly one place a key reaches the wire (ADR-0021 §3).
- A provider with no key set lists nothing, and says so. This is the same visible-before-you-send principle the Status column already applies.

## Alternatives considered

| Option | Why rejected |
|--------|--------------|
| A curated static catalogue per vendor | Rejected: model ids move faster than releases, and a stale menu of models that no longer exist is worse than a text field. Kept as the fallback that already exists — the field stays typeable. |
| A non-editable dropdown | Rejected: it makes an unreachable model out of every preview id, fine-tune and gateway alias, and turns a failed fetch into a broken feature. |
| Fetching on startup, or on every repaint | Rejected: an unrequested outbound request to a third party is not something opening an editor consents to. |
| Model choice per provider, edited from the header | Rejected: it rewrites a persisted global setting every time someone tries a model once, which is the problem this ADR exists to fix. |
