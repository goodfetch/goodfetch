// Structural Activepieces piece framework contracts matching
// `@activepieces/pieces-framework` (ADRs 0027, 0070, 0090, 0095; spec #608,
// issue 1409).

export interface PieceProperty {
  readonly displayName: string;
  readonly description?: string;
  readonly type: "SHORT_TEXT" | "SECRET_TEXT" | "JSON" | "STATIC_MULTI_SELECT_DROPDOWN";
  readonly required: boolean;
  readonly defaultValue?: unknown;
  readonly options?: ReadonlyArray<{ readonly label: string; readonly value: string }>;
}

export type PropertyMap = Readonly<Record<string, PieceProperty>>;

export interface ActivepiecesCollieAuth {
  readonly baseUrl: string;
  readonly token: string;
  readonly protocolVersion?: string;
}

export interface PieceAuthDefinition<TAuth> {
  readonly type: "CUSTOM_AUTH";
  readonly displayName: string;
  readonly description: string;
  readonly required: true;
  readonly props: PropertyMap;
  validate(context: {
    readonly auth: TAuth;
  }): Promise<{ readonly valid: true } | { readonly valid: false; readonly error: string }>;
}

export interface PieceStore {
  get(key: string): Promise<unknown>;
  put<T>(key: string, value: T): Promise<T>;
  delete(key: string): Promise<void>;
}

export interface ActionContext<TAuth, TProps extends Record<string, unknown>> {
  readonly auth: TAuth;
  readonly propsValue: TProps;
  readonly store: PieceStore;
  readonly fetchImpl?: typeof fetch;
}

export interface ActionDefinition<TAuth, TProps extends Record<string, unknown>> {
  readonly name: string;
  readonly displayName: string;
  readonly description: string;
  readonly auth: PieceAuthDefinition<TAuth>;
  readonly props: PropertyMap;
  run(context: ActionContext<TAuth, TProps>): Promise<Record<string, unknown>>;
}

export interface TriggerHookContext<TAuth, TProps extends Record<string, unknown>> {
  readonly auth: TAuth;
  readonly propsValue: TProps;
  readonly webhookUrl: string;
  readonly store: PieceStore;
  readonly fetchImpl?: typeof fetch;
}

export interface TriggerPayload {
  readonly body: unknown;
  readonly rawBody?: string | Uint8Array;
  readonly headers: Record<string, string | string[] | undefined>;
}

export interface TriggerPayloadContext<TAuth, TProps extends Record<string, unknown>> {
  readonly auth: TAuth;
  readonly propsValue: TProps;
  readonly payload: TriggerPayload;
  readonly store: PieceStore;
}

export interface TriggerDefinition<TAuth, TProps extends Record<string, unknown>> {
  readonly name: string;
  readonly displayName: string;
  readonly description: string;
  readonly type: "WEBHOOK";
  readonly auth: PieceAuthDefinition<TAuth>;
  readonly props: PropertyMap;
  readonly sampleData: Record<string, unknown>;
  onEnable(context: TriggerHookContext<TAuth, TProps>): Promise<void>;
  onDisable(context: TriggerHookContext<TAuth, TProps>): Promise<void>;
  run(context: TriggerPayloadContext<TAuth, TProps>): Promise<readonly Record<string, unknown>[]>;
}

export interface PieceActionSummary<TAuth> {
  readonly name: string;
  readonly displayName: string;
  readonly description: string;
  readonly auth: PieceAuthDefinition<TAuth>;
  readonly props: PropertyMap;
}

export interface PieceTriggerSummary<TAuth> {
  readonly name: string;
  readonly displayName: string;
  readonly description: string;
  readonly type: "WEBHOOK";
  readonly auth: PieceAuthDefinition<TAuth>;
  readonly props: PropertyMap;
  readonly sampleData: Record<string, unknown>;
}

export interface PieceDefinition<TAuth> {
  readonly name: string;
  readonly displayName: string;
  readonly description: string;
  readonly auth: PieceAuthDefinition<TAuth>;
  readonly actions: readonly PieceActionSummary<TAuth>[];
  readonly triggers: readonly PieceTriggerSummary<TAuth>[];
}

export function createMemoryStore(): PieceStore {
  const entries = new Map<string, unknown>();
  return {
    async get(key: string): Promise<unknown> {
      return entries.has(key) ? entries.get(key) : null;
    },
    async put<T>(key: string, value: T): Promise<T> {
      entries.set(key, value);
      return value;
    },
    async delete(key: string): Promise<void> {
      entries.delete(key);
    },
  };
}

export function createMockActionContext<TAuth, TProps extends Record<string, unknown>>(options: {
  readonly auth: TAuth;
  readonly propsValue: TProps;
  readonly store?: PieceStore;
  readonly fetchImpl?: typeof fetch;
}): ActionContext<TAuth, TProps> {
  return {
    auth: options.auth,
    propsValue: options.propsValue,
    store: options.store ?? createMemoryStore(),
    ...(options.fetchImpl === undefined ? {} : { fetchImpl: options.fetchImpl }),
  };
}
