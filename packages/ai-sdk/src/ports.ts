/**
 * What an application supplies at the provider boundary.
 *
 * Both ports exist because the SDK deliberately owns neither concern. It does
 * not store secrets and it does not decide which models a person may reach —
 * those are consent, policy and persistence decisions that belong to the
 * application, and an SDK default would quietly override them.
 *
 * These mirror `ai-providers`' `AiCredentialSource` and `AiModelAuthority` in
 * Rust, including the rule that the credential resolves *before* any socket is
 * opened.
 */

/** Why an application declined to produce a credential. Closed, not free text. */
export type CredentialRefusal =
  /** No credential is configured for this provider. */
  | 'missing'
  /** A credential exists but the application will not release it right now. */
  | 'withheld'

export type CredentialResult =
  | { readonly ok: true; readonly apiKey: string }
  | { readonly ok: false; readonly refusal: CredentialRefusal; readonly message: string }

/**
 * Resolves a runtime secret. Stores nothing.
 *
 * Called once per run. It is not called again on a 429 or a 401: reacting to
 * those is key rotation, which needs an execution owner this SDK does not have.
 *
 * The resolved key is passed to the underlying provider factory and is never
 * placed in a request, an error, or anything serializable.
 */
export type CredentialSource = {
  resolve(providerId: string): Promise<CredentialResult> | CredentialResult
}

/**
 * Decides whether a model may be reached.
 *
 * Consulted before the credential and before the network, so a model the
 * application has not approved never reaches a provider — and a refusal costs
 * nothing and discloses nothing.
 */
export type ModelAuthority = {
  permits(providerId: string, modelId: string): boolean
}

/** Permits every model. For tests and local playgrounds, never a default. */
export function permitAll(): ModelAuthority {
  return { permits: () => true }
}

/** Reads a key from a caller-supplied string. The application decides where it came from. */
export function staticCredential(apiKey: string): CredentialSource {
  return {
    resolve: () =>
      apiKey === ''
        ? { ok: false, refusal: 'missing', message: 'no api key was configured for this provider' }
        : { ok: true, apiKey },
  }
}
