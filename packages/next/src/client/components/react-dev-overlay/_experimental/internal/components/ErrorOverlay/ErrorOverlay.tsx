import type { VersionInfo } from '../../../../../../../server/dev/parse-version-info'
import { Dialog, DialogHeader, DialogBody, DialogContent } from '../Dialog'
import { Overlay } from '../Overlay'
import { VersionStalenessInfo } from '../VersionStalenessInfo'

export function ErrorOverlay({
  errorType,
  errorMessage,
  versionInfo,
  children,
}: {
  errorType: string
  errorMessage: string
  children: React.ReactNode
  versionInfo?: VersionInfo
}) {
  return (
    <Overlay>
      <Dialog
        type="error"
        aria-labelledby="nextjs__container_errors_label"
        aria-describedby="nextjs__container_errors_desc"
      >
        <DialogContent>
          <DialogHeader className="nextjs-container-errors-header">
            <h1
              id="nextjs__container_errors_label"
              className="nextjs__container_errors_label"
            >
              {errorType}
            </h1>
            <VersionStalenessInfo versionInfo={versionInfo} />
            <p
              id="nextjs__container_errors_desc"
              className="nextjs__container_errors_desc"
            >
              {errorMessage}
            </p>
          </DialogHeader>
          <DialogBody className="nextjs-container-errors-body">
            {children}
          </DialogBody>
        </DialogContent>
      </Dialog>
    </Overlay>
  )
}
