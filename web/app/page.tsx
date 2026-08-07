import { headers } from 'next/headers'
import { redirect } from 'next/navigation'

import { negotiate } from '@/lib/i18n/config'

/**
 * The bare root picks a language and redirects.
 *
 * Negotiated from `Accept-Language` rather than defaulting, because an Algerian reader arriving at
 * an English page has been told the product is not for them before they read a word.
 */
export default async function RootRedirect() {
  const locale = negotiate((await headers()).get('accept-language'))
  redirect(`/${locale}`)
}
