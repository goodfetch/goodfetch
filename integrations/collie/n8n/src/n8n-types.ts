// Structural n8n node and credential contracts matching `n8n-workflow` v1
// (ADRs 0027, 0070, 0090, 0095). Kept as zero-runtime-dependency TypeScript
// types so `@goodfetch/n8n-nodes-collie` satisfies n8n's verified-node rules.

export type IDataObject = Record<string, unknown>;

export interface INodePropertyOptions {
  readonly name: string;
  readonly value: string;
  readonly description?: string;
  readonly action?: string;
}

export interface INodeProperties {
  readonly displayName: string;
  readonly name: string;
  readonly type:
    | "string"
    | "number"
    | "boolean"
    | "options"
    | "multiOptions"
    | "json"
    | "collection";
  readonly default: unknown;
  readonly required?: boolean;
  readonly noDataExpression?: boolean;
  readonly description?: string;
  readonly placeholder?: string;
  readonly typeOptions?: IDataObject;
  readonly displayOptions?: {
    readonly show?: Record<string, readonly unknown[]>;
    readonly hide?: Record<string, readonly unknown[]>;
  };
  readonly options?: readonly INodePropertyOptions[];
}

export interface IAuthenticateGeneric {
  readonly type: "generic";
  readonly properties: {
    readonly headers?: Record<string, string>;
  };
}

export interface ICredentialTestRequest {
  readonly request: {
    readonly baseURL?: string;
    readonly url: string;
    readonly method?: "GET" | "POST" | "PUT" | "DELETE";
    readonly headers?: Record<string, string>;
    readonly body?: IDataObject;
  };
}

export interface ICredentialType {
  readonly name: string;
  readonly displayName: string;
  readonly documentationUrl?: string;
  readonly properties: readonly INodeProperties[];
  readonly authenticate?: IAuthenticateGeneric;
  readonly test?: ICredentialTestRequest;
}

export interface IWebhookDescription {
  readonly name: "default";
  readonly httpMethod: "POST";
  readonly responseMode: "onReceived";
  readonly path: string;
}

export interface INodeTypeDescription {
  readonly displayName: string;
  readonly name: string;
  readonly group: readonly string[];
  readonly version: number;
  readonly subtitle?: string;
  readonly description: string;
  readonly defaults: {
    readonly name: string;
  };
  readonly inputs: readonly string[];
  readonly outputs: readonly string[];
  readonly usableAsTool?: boolean;
  readonly credentials?: ReadonlyArray<{
    readonly name: string;
    readonly required?: boolean;
  }>;
  readonly webhooks?: readonly IWebhookDescription[];
  readonly properties: readonly INodeProperties[];
}

export interface INodeExecutionData {
  readonly json: IDataObject;
  readonly pairedItem?: { readonly item: number };
}

export interface IHttpRequestOptions {
  readonly method: "POST" | "GET";
  readonly url: string;
  readonly headers?: Record<string, string>;
  readonly body?: IDataObject;
  readonly json?: boolean;
  readonly returnFullResponse?: boolean;
  readonly ignoreHttpStatusErrors?: boolean;
}

export interface IHttpFullResponse {
  readonly statusCode: number;
  readonly body: unknown;
  readonly headers?: Record<string, string | string[] | undefined>;
}

export interface INodeHelpers {
  httpRequest(options: IHttpRequestOptions): Promise<unknown>;
  returnJsonArray?(items: readonly IDataObject[]): INodeExecutionData[];
}

export interface INodeDescriptor {
  readonly id?: string;
  readonly name: string;
  readonly type: string;
  readonly typeVersion: number;
}

export interface IExecuteFunctions {
  getInputData(): readonly INodeExecutionData[];
  getNodeParameter(name: string, itemIndex: number, fallbackValue?: unknown): unknown;
  getCredentials(type: string): Promise<IDataObject>;
  getNode(): INodeDescriptor;
  continueOnFail(): boolean;
  readonly helpers: INodeHelpers;
}

export interface IHookFunctions {
  getNodeWebhookUrl(name: string): string | undefined;
  getNodeParameter(name: string, fallbackValue?: unknown): unknown;
  getCredentials(type: string): Promise<IDataObject>;
  getWorkflowStaticData(type: "node" | "global"): IDataObject;
  getNode(): INodeDescriptor;
  readonly helpers: INodeHelpers;
}

export interface IWebhookRequest {
  readonly rawBody?: Uint8Array | string;
  readonly body?: unknown;
  readonly headers?: Record<string, string | string[] | undefined>;
}

export interface IWebhookResponseData {
  readonly workflowData?: readonly INodeExecutionData[][];
}

export interface IWebhookFunctions {
  getRequestObject(): IWebhookRequest;
  getBodyData(): IDataObject;
  getHeaderData(): Record<string, string | string[] | undefined>;
  getNodeParameter(name: string, fallbackValue?: unknown): unknown;
  getWorkflowStaticData(type: "node" | "global"): IDataObject;
  getNode(): INodeDescriptor;
  readonly helpers: INodeHelpers;
}

export interface INodeType {
  readonly description: INodeTypeDescription;
  execute?(this: IExecuteFunctions): Promise<INodeExecutionData[][]>;
  webhookMethods?: {
    readonly default: {
      checkExists(this: IHookFunctions): Promise<boolean>;
      create(this: IHookFunctions): Promise<boolean>;
      delete(this: IHookFunctions): Promise<boolean>;
    };
  };
  webhook?(this: IWebhookFunctions): Promise<IWebhookResponseData>;
}
