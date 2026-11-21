export interface StartOptions {
  runId?: string;
}

export interface Subscription extends Disposable {
  close(): void;
}
