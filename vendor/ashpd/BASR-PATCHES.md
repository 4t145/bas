# Local changes to ashpd 0.13.13

`src/proxy.rs`: do not cache or query properties on
`org.freedesktop.portal.Request`. This short-lived interface defines `Close`
and `Response`, with no properties or version. Querying `GetAll` before its
object exists produces a zbus warning during the file chooser request.

Other portal interfaces retain their normal property cache and version checks.

Interface: https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Request.html
