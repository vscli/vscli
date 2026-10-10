# Actual Windows boolean-child save difference

Inputs and observed evidence are retained unchanged under `/tmp/vscli-save-readiness-qualified-remote-artifacts`. Remote run 38053830713 completed on all three platforms, candidate revision 2c5ee40fe0ba3cf3ff7112c0cf7bb71d3987515b. Parent independently verified nine source bytes and all capture digests. Windows trace SHA 98c428a4e2036daa4cebfd9bfef0ccf9a3d523b46be9c66dc6435ef9fddee629, evidence SHA 4faed9e1786d65eb91547fc2de3abebeed95dd281934ea77e0285b3d8b28ac87.

## Actual facts

Only ancestor-false-child differs. Setup effective/inspect on every platform reports source.fixAll explicit and source.fixAll.child boolean false. Warm provider call, exact target callback request/source.fixAll/triggerKind2/version3/text/returned kinds are equal, cancellation false. Windows durably saves fix=1 but child=0, modelversion4; Linux/Mac save fix=1 and child=1, version5. Undo/Redo preserves the respective missing/present child transaction. Auxiliary save formatter callback positively witnesses participant installation on all platforms; this is not the earlier missing-participant case. No target-time configuration snapshot or migration event is present. No capture was rerun and no golden was rewritten.

## Pinned primary source and inference

The native source action exclusion branch excludes only the literal string never, while boolean false is merely omitted from requested kinds. This permits a false child under an explicit ancestor before migration. The same pinned version has an editor settings migration converting every boolean codeActionsOnSave entry to explicit/never, applied asynchronously by an Eventually workbench contribution. Hence false child becomes never child after migration, which changes exclusion under the ancestor. These are source facts; Windows migration between setup and target save is a strong explanation, not directly observed proof. No platform branch is needed to explain the timing difference.

Sources (all exact 1.95 product commit 912bb683695358a54ae0c670461738984cbb5b95):
- [Save participant, lines313-344](https://github.com/microsoft/vscode/blob/912bb683695358a54ae0c670461738984cbb5b95/src/vs/workbench/contrib/codeEditor/browser/saveParticipants.ts#L313-L344).
- [Boolean migration, lines181-195](https://github.com/microsoft/vscode/blob/912bb683695358a54ae0c670461738984cbb5b95/src/vs/editor/browser/config/migrateOptions.ts#L181-L195).
- [Registration bridge, lines7-16](https://github.com/microsoft/vscode/blob/912bb683695358a54ae0c670461738984cbb5b95/src/vs/workbench/contrib/codeEditor/browser/editorSettingsMigration.ts#L7-L16).
- [Asynchronous migration and writes, lines78-143](https://github.com/microsoft/vscode/blob/912bb683695358a54ae0c670461738984cbb5b95/src/vs/workbench/common/configuration.ts#L78-L143).
- [Eventually registration, line20](https://github.com/microsoft/vscode/blob/912bb683695358a54ae0c670461738984cbb5b95/src/vs/workbench/browser/workbench.contribution.ts#L20).

## Proper qualification response

Do not alter native policy from this single transitional observation or change the frozen false fixture to never silently. Preserve the complete Windows cohort as genuine differing evidence. Formatter readiness proves the Restored contribution, not completion of Eventually settings migration. An independently versioned diagnostic observer should record resource/language effective and inspect values plus configuration-change events at provider callback and didSave (with bounded full values); target commands remain once-only. This could establish whether exclusion at save used a migrated never value even though setup saw false. A stable post-migration qualification needs explicitly labeled setup input normalization/readiness and fresh actual observations; it must not await child edit or retry Save until a preferred result appears. Alternatively a separately named never-input cohort qualifies stable exclusion, while original boolean startup traces stay preserved and explicitly transitional.

Current strict auxiliary validation is metadata-only and may pass all platforms without claiming target equality. Existing original default baseline and policy/text consumer remain unchanged pending parent adoption decision. No local VS Code, build/test, Git mutation or repository write occurred during this diagnosis.
