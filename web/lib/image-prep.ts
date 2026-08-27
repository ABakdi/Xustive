/**
 * What leaves the browser when someone searches by image (M3-T06.2, shared with M10).
 *
 * The picture is decoded, rotated upright, downscaled to ≤ 2048 px on the long edge and
 * re-encoded as JPEG through a canvas — which drops every EXIF field, GPS included. The server
 * cannot leak coordinates it never receives.
 */
export const MAX_DIM = 2048

export async function prepareImage(file: Blob): Promise<Blob> {
  const bitmap = await createImageBitmap(file, { imageOrientation: 'from-image' })
  const longEdge = Math.max(bitmap.width, bitmap.height)
  const scale = Math.min(1, MAX_DIM / longEdge)
  const width = Math.round(bitmap.width * scale)
  const height = Math.round(bitmap.height * scale)

  const canvas = document.createElement('canvas')
  canvas.width = width
  canvas.height = height
  const ctx = canvas.getContext('2d')
  if (!ctx) {
    bitmap.close()
    return file
  }
  ctx.drawImage(bitmap, 0, 0, width, height)
  bitmap.close()

  return new Promise<Blob>((resolve) => {
    // Fall back to the original bytes if the browser cannot encode, rather than failing the flow.
    canvas.toBlob((blob) => resolve(blob ?? file), 'image/jpeg', 0.92)
  })
}

/** The key under which the OCR page hands a prepared picture to the reverse-image page. */
export const HANDOFF_KEY = 'xustive:reverse-image'

/** Stash a prepared picture for the next page, as a data URL; read once and cleared there. */
export async function handOff(blob: Blob): Promise<boolean> {
  try {
    const data = await new Promise<string>((resolve, reject) => {
      const r = new FileReader()
      r.onload = () => resolve(String(r.result))
      r.onerror = () => reject(r.error)
      r.readAsDataURL(blob)
    })
    sessionStorage.setItem(HANDOFF_KEY, data)
    return true
  } catch {
    return false
  }
}

/** The picture the previous page handed off, if any — taken, not copied. */
export async function takeHandOff(): Promise<Blob | null> {
  try {
    const data = sessionStorage.getItem(HANDOFF_KEY)
    if (!data) return null
    sessionStorage.removeItem(HANDOFF_KEY)
    const res = await fetch(data)
    return await res.blob()
  } catch {
    return null
  }
}
