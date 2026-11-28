// Goodfetch Collie HTTP listener credentials for n8n (ADRs 0017, 0027, 0090,
// 0095; spec #608, issue 1405).
import type {
  IAuthenticateGeneric,
  ICredentialTestRequest,
  ICredentialType,
  Icon,
  INodeProperties,
} from "n8n-workflow";
import {
  COLLIE_FORM,
  COLLIE_PRODUCT,
  DEFAULT_PROTOCOL_VERSION,
  RPC_PATH,
} from "../nodes/GoodfetchCollie/rpc.ts";

export class GoodfetchCollieApi implements ICredentialType {
  readonly name = "goodfetchCollieApi";
  readonly displayName = "Goodfetch Collie API";
  readonly icon: Icon = {
    light: "file:goodfetchCollie.svg",
    dark: "file:goodfetchCollie.dark.svg",
  };
  readonly documentationUrl = "https://docs.goodfetch.dev/integrations/n8n";
  readonly properties: INodeProperties[] = [
    {
      displayName: "Base URL",
      name: "baseUrl",
      type: "string",
      default: "http://127.0.0.1:7801",
      required: true,
      placeholder: "http://127.0.0.1:7801",
      description: "Base URL of the Collie Daemon HTTP JSON-RPC listener",
    },
    {
      displayName: "Bearer Token",
      name: "token",
      type: "string",
      typeOptions: { password: true },
      default: "",
      required: true,
      description: "Bearer token configured on the Collie Daemon HTTP listener",
    },
    {
      displayName: "Protocol Version",
      name: "protocolVersion",
      type: "string",
      default: DEFAULT_PROTOCOL_VERSION,
      description: "Goodfetch protocol minor version sent in the Goodfetch-Protocol header",
    },
  ];

  readonly authenticate: IAuthenticateGeneric = {
    type: "generic",
    properties: {
      headers: {
        Authorization: "=Bearer {{$credentials.token}}",
        "Goodfetch-Protocol": "={{$credentials.protocolVersion || '0.1'}}",
        "Goodfetch-Product": COLLIE_PRODUCT,
        "Goodfetch-Form": COLLIE_FORM,
      },
    },
  };

  readonly test: ICredentialTestRequest = {
    request: {
      baseURL: "={{$credentials.baseUrl}}",
      url: RPC_PATH,
      method: "POST",
      headers: {
        "Content-Type": "application/json",
      },
      body: {
        jsonrpc: "2.0",
        id: 1,
        method: "daemon.status",
        params: {},
      },
    },
  };
}
