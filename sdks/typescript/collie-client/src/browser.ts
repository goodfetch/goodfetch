// The client package's runtime for a browser (ADR 0068, item 2; spec #592):
// the protocol code the TypeScript SDK runs, with no Node built-in, which
// a Client in a browser is built on. Request correlation and `$/cancel`
// (`Channel`), framing, the event stream's credits and resumption from a
// position (`subscribeEvents`), the errors a reply raises, and the protocol
// and Product versions `hello` names. Finding and starting an engine stays
// in the root entry, which needs Node.
export { Channel } from "./rpc.ts";
export type {
  ChannelCallOptions,
  ChannelOpenOptions,
  ChannelOwner,
  OpenCall,
  Transport,
  TransportEnd,
} from "./rpc.ts";
export { encodeMessage, parseLine } from "./framing.ts";
export type { Message, MessageId } from "./framing.ts";
export { subscribeEvents } from "./stream.ts";
export type {
  EventsSubscription,
  EventsSubscriptionOptions,
  LastRead,
  StreamedEvent,
} from "./stream.ts";
export { errorOfReply } from "./errors.ts";
export type { ErrorClasses } from "./errors.ts";
export { PRODUCT_VERSION } from "./generated/connection.ts";
