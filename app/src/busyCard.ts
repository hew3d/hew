/**
 * The busy card's shared words. The card runs one long change at a time;
 * a second one asked for while it is up does not run, and says so.
 */
import { msg } from '@lingui/core/macro'

/** The toast when a change could not start because another long change is
 *  still running behind the busy card. */
export const BUSY_REFUSED = msg`Another change is still running. Press Enter or click again when it finishes.`
