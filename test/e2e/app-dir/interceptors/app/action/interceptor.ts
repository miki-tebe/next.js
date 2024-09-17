import { setTimeout } from 'timers/promises'
import { NextRequest } from 'next/server'
import { logWithTime } from '../time-utils'

export default async function interceptRoot(
  request: NextRequest
): Promise<void> {
  await logWithTime('ActionInterceptor', () => setTimeout(250))
}