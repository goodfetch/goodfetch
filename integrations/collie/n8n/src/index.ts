// Public entry point for `@goodfetch/n8n-nodes-collie` (ADRs 0027, 0070, 0090,
// 0095; spec #608, issue 1405).
export { GoodfetchCollieApi } from "./credentials/GoodfetchCollieApi.credentials.ts";
export { GoodfetchCollie } from "./nodes/GoodfetchCollie/GoodfetchCollie.node.ts";
export { GoodfetchCollieTrigger } from "./nodes/GoodfetchCollie/GoodfetchCollieTrigger.node.ts";
export {
  callCollieRpc,
  COLLIE_FORM,
  COLLIE_PRODUCT,
  DEFAULT_PROTOCOL_VERSION,
  fixForCollieError,
  mapCatalogErrorToN8n,
  N8nCollieError,
  normalizeGroup,
  parseCredentials,
  parseJsonObjectParam,
  parseRpcErrorResponse,
  RPC_PATH,
  verifyStandardWebhookSignature,
  type CollieCredentials,
  type CollieErrorGroup,
  type N8nCollieErrorInfo,
} from "./rpc.ts";
export type {
  IAuthenticateGeneric,
  ICredentialTestRequest,
  ICredentialType,
  IDataObject,
  IExecuteFunctions,
  IHookFunctions,
  IHttpFullResponse,
  IHttpRequestOptions,
  INodeDescriptor,
  INodeExecutionData,
  INodeHelpers,
  INodeProperties,
  INodePropertyOptions,
  INodeType,
  INodeTypeDescription,
  IWebhookDescription,
  IWebhookFunctions,
  IWebhookRequest,
  IWebhookResponseData,
} from "./n8n-types.ts";
