# Rules and terms for AI-generated mod assets

What Paradox, Steam and the image providers say about AI-generated art in Hearts of Iron IV mods, and what this tool does about it. Researched 2026-11-01. This is a summary for modders, not legal advice. Sources that could not be read are marked **unverified**: check them yourself before you rely on them.

## Paradox Interactive

- **The User Agreement (EULA)** at <https://legal.paradoxplaza.com/eula> (updated 2026-01-21) treats mods as user-generated content:
  - The creator is responsible for the mod, and it must be non-commercial and freely available. Donations and sponsorships are allowed; paywalls are not.
  - It must not infringe anyone's rights.
  - You keep rights only in the new material you add, and you grant Paradox a broad licence to it.
  - **It says nothing about AI-generated content**, for or against.
- **Paradox's older forum rules for mods** (2018) and the forum thread on AI art in mods could not be read (server error and browser check). **Unverified.** A secondary snippet says mods that use Paradox content must be usable only by owners of the game. Do not redistribute vanilla art outside a mod.
- **The Paradox Mods platform terms** could not be found or read. **Unverified.**

## Steam and Valve

- **The Steamworks content survey** (introduced January 2024, revised since) requires disclosure of AI content in games and apps submitted through Steamworks. It does not mention Workshop items, and no Workshop-specific AI rule was found.
- **The Steam Subscriber Agreement** at <https://store.steampowered.com/subscriber_agreement/> (updated 2026-09-10), section 6:
  - You license your uploaded content to Valve.
  - You **warrant that you have the rights to it, including personal rights**, and that you created it or may submit it.
  - Valve can remove Workshop items for any reason.

## Image providers

| Provider | Output | Points to note |
|---|---|---|
| OpenAI (API and ChatGPT) | Assigned to you "if any" rights exist. Output may not be unique. | The terms forbid automated extraction of output outside the API and sharing credentials. The pages returned 403 to automated reading, so the wording was checked only through search excerpts. **Unverified.** Rules on images of real people have changed over time. Check the current usage policies. |
| Google Gemini API | Google claims no ownership, may generate similar output for others, and you are responsible for use (terms effective 2026-03-23). | Free-tier prompts and outputs may be used to improve products. Users must be 18 or over. |
| fal.ai | No clear assignment; fal disclaims any IP claim (terms of 2026-09-08). | Each model has its own licence. No resale. |
| Local models (ComfyUI) | Depends on the model's licence. | FLUX.1 [dev] weights are non-commercial. The Stability Community License is free under about USD 1M yearly revenue. |

In the United States, wholly AI-generated output is not copyrightable (US Copyright Office report, Part 2, 2025-01-29). Human modification or arrangement can be. Other countries differ.

## What this means for a mod, and what the tool does

1. **No ban was found.** The binding rules are the general ones: keep the mod free, do not infringe, and only upload what you have the right to upload.
2. **Disclosure.** Paradox and Steam do not require it for mods, as far as could be found, but players often expect it. Every asset's ledger records the provider, model, date and prompt, and `hoi4.image_ledger` can produce a list for the mod's description. Disclosing is recommended.
3. **Real people.** HOI4 portraits show real historical leaders, and the Steam warranty covers personal rights. Providers may refuse, or treat differently, images of real people. The portrait tools warn about this. Prefer stylised portraits, and do not edit photographs of identifiable living people.
4. **Vanilla art as references.** The maintainer decided on 2026-11-01 that vanilla references may always be sent to providers when they help match the style. The EULA gives you no rights in Paradox's art, so:
   - The tool never copies vanilla files into a mod.
   - It records which references were sent with each request.
   - Assets must be new works in the vanilla style, not near-copies. The review rubric asks for this.
5. **The ChatGPT login.** It uses your own plan through a backend OpenAI does not document for other tools. It is local only, and hosted HOI4 Factory never uses it (see [the ChatGPT login](chatgpt-login.md)). The credential-sharing and automated-extraction clauses are the reasons it stays a personal, local route.
6. **Selling.** Paradox requires mods to be free. Do not sell generated assets as a paid mod. Also check each provider's resale rules.

## Sources

- Paradox User Agreement: <https://legal.paradoxplaza.com/eula>
- Steamworks content survey: <https://partner.steamgames.com/doc/gettingstarted/contentsurvey>
- Steam Subscriber Agreement: <https://store.steampowered.com/subscriber_agreement/>
- OpenAI Terms of Use: <https://openai.com/policies/terms-of-use/> (**unverified**: not readable automatically)
- Gemini API terms: <https://ai.google.dev/gemini-api/terms>
- fal terms: <https://fal.ai/terms>
- FLUX.1 [dev] licence: <https://bfl.ai/legal/non-commercial-license-terms>
- Stability AI licence: <https://stability.ai/license>
