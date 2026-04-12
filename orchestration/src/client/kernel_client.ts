/**
 * gRPC client for the Eigenius kernel service.
 *
 * Wraps the four kernel operations: Load, Query, Validate, Reflect.
 * Architecture reference: §2.2
 */

export class KernelClient {
  private endpoint: string;

  constructor(endpoint: string) {
    this.endpoint = endpoint;
  }

  load(_resources: unknown): Promise<unknown> {
    // TODO: gRPC call to kernel Load RPC
    return Promise.reject(
      new Error(`Not implemented (endpoint: ${this.endpoint})`),
    );
  }

  query(_eigenql: string): Promise<unknown> {
    // TODO: gRPC call to kernel Query RPC
    return Promise.reject(new Error("Not implemented"));
  }

  validate(_dagSpec: unknown): Promise<unknown> {
    // TODO: gRPC call to kernel Validate RPC
    return Promise.reject(new Error("Not implemented"));
  }

  reflect(_trace: unknown): Promise<unknown> {
    // TODO: gRPC call to kernel Reflect RPC
    return Promise.reject(new Error("Not implemented"));
  }
}
