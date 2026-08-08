import { cookies } from 'next/headers'

/**
 * Which instant-answer tools the reader has switched off.
 *
 * # Why this is a cookie and not a query parameter
 *
 * The obvious design sends the disabled list to the API so a suppressed tool is never computed.
 * It is the wrong one. The set of tools a person has turned off is small, stable, and unusual
 * enough to identify them across requests — it would be a fingerprint handed to the one component
 * that currently receives no preference data at all.
 *
 * So the API answers as it always does and this layer drops what the reader does not want. The
 * cost is an answer computed and discarded over an internal hop inside the serving plane. The
 * benefit is that opting out of a tool cannot make you more identifiable, which would be a
 * perverse thing for a privacy control to do.
 *
 * # Why a cookie and not localStorage
 *
 * The card is server-rendered. A preference the server cannot read means rendering the card and
 * then removing it in an effect, so a tool you switched off flashes on screen every single search
 * — which is worse than not having the control.
 */
export const TOOLS_COOKIE = 'xustive-tools-off'

/**
 * Cap on how many identifiers are stored.
 *
 * Cookies go on every request. A malformed or hostile value should cost bytes, not kilobytes, and
 * there is no plausible reason to disable more tools than exist.
 */
const MAX_DISABLED = 32

/** Cap on a single identifier, matching the API's `[a-z-]+` shape. */
const MAX_ID_LENGTH = 32

/**
 * Parse the cookie value into a set.
 *
 * Deliberately total: a cookie is user-editable and arrives from the network, so anything
 * unparseable resolves to "nothing disabled" rather than throwing. Failing open is right here —
 * the failure shows a card the reader did not want, which they can dismiss again. Failing closed
 * would hide every tool with no way to discover why.
 */
export function parseDisabled(value: string | undefined): Set<string> {
  if (!value) return new Set()
  return new Set(
    value
      .split(',')
      .map((id) => id.trim().toLowerCase())
      .filter((id) => id.length > 0 && id.length <= MAX_ID_LENGTH && /^[a-z-]+$/.test(id))
      .slice(0, MAX_DISABLED),
  )
}

export function serialiseDisabled(ids: Iterable<string>): string {
  // Sorted so the same set always produces the same cookie. An unstable value would churn the
  // header on every write for no reason.
  return [...new Set(ids)].sort().slice(0, MAX_DISABLED).join(',')
}

export async function readDisabledTools(): Promise<Set<string>> {
  return parseDisabled((await cookies()).get(TOOLS_COOKIE)?.value)
}

/** The tool inventory, as served by the API. */
export type ToolInfo = { id: string; keyword: string }
