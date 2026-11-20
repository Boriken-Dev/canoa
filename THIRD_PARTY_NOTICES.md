# Third-party notices

## Microsoft Office JavaScript (Office.js) API Library

Canoa's host carries the Microsoft Office JavaScript (Office.js) API Library, unmodified, from
[github.com/OfficeDev/office-js](https://github.com/OfficeDev/office-js) (commit `498e88261bc578243d40dd79083953d86f84e7f6`): the files the Office add-in's task pane
loads, and the repository's license. It is used under Microsoft's license terms for that library, in
[licenses/OFFICE_JS_LICENSE.md](licenses/OFFICE_JS_LICENSE.md); a running host serves the same text at `/office/js/LICENSE.md`.

Microsoft is not responsible for maintaining, updating or supporting Canoa.

A build made with `CANOA_OFFICE_JS=cdn` carries none of it and names Microsoft's CDN instead.
