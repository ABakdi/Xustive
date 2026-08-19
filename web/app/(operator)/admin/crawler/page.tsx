import { redirect } from 'next/navigation'

/** The live view was `/admin/crawler` in the old Rust-rendered console. Keep that URL working —
 * bookmarks and muscle memory point at it — by redirecting to its new home. */
export default function CrawlerRedirect() {
  redirect('/admin/live')
}
