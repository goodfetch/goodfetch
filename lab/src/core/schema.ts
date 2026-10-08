import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { createRequire } from 'node:module';
import { labRoot } from '../registry.ts';

type Validate = ((data: unknown) => boolean) & { errors?: { instancePath: string; message?: string }[] | null };
const require = createRequire(import.meta.url);
const Ajv2020 = require('ajv/dist/2020.js') as new (options: object) => { compile: (schema: object) => Validate };
// strictRequired is off because conditional `then.required` names properties declared at the top level.
const ajv = new Ajv2020({ allErrors: true, strict: true, strictRequired: false, allowUnionTypes: true });
const cache = new Map<string, Validate>();

export const schemaNames = ['decision', 'claim', 'metric-catalog', 'run.v2', 'ledger', 'classification', 'campaign-bench.v2'] as const;
export type SchemaName = typeof schemaNames[number];

export function validateDocument(name: SchemaName, document: unknown): string[] {
  let validate = cache.get(name);
  if (!validate) { validate = ajv.compile(JSON.parse(readFileSync(join(labRoot, 'schemas', `${name}.schema.json`), 'utf8'))); cache.set(name, validate); }
  return validate(document) ? [] : (validate.errors ?? []).map(error => `${error.instancePath || '/'} ${error.message ?? 'invalid'}`);
}
