// Engineering-owned Svelte preset. Rules, ignores, and the Prettier
// reconciliation live in @jrmoulckers/eslint-config; see
// https://github.com/jrmoulckers/engineering/blob/main/docs/adopting.md
//
// Nothing is extended locally: libro's previous configuration was a strict
// subset of this preset, so adopting it loses no rule.
import { svelteConfig } from '@jrmoulckers/eslint-config/svelte';

export default svelteConfig();
