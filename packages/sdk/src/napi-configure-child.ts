import { configureNapi } from './mod.ts';
import { loadNapi, type NapiAddon } from './napi.ts';

const addon = {
	version: () => ({ version: 'embedded' }),
} as NapiAddon;

configureNapi(addon);
console.log((loadNapi().version() as { version: string }).version);
