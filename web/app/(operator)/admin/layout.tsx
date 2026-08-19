import type { Metadata } from 'next'

import { AdminSidebar } from '@/components/admin/AdminSidebar'

export const metadata: Metadata = {
  title: 'Xustive admin',
  robots: { index: false, follow: false },
}

/**
 * The operator console shell. A two-column grid — sidebar and content — the same shape the Rust
 * process used to render, now a set of ordinary Next.js pages that read the `/api/v1/admin/*` JSON
 * API. Deliberately plainer than the search UI: it is a tool, always LTR and in English.
 */
export default function AdminLayout({ children }: { children: React.ReactNode }) {
  return (
    <div
      dir="ltr"
      className="mx-auto grid max-w-[1400px] gap-8 px-6 pb-16 pt-6"
      style={{ gridTemplateColumns: '190px minmax(0, 1fr)', color: 'var(--fg)' }}
    >
      <AdminSidebar />
      <main className="min-w-0">{children}</main>
    </div>
  )
}
