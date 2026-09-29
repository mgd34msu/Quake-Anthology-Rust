//! QVM trap and export numbers for the game, client-game, and UI modules.
//!
//! Port of `src/compat/qvm/abi.ts` (from Quake III Arena's `code/game/g_public.h`,
//! `cgame/cg_public.h`, `ui/ui_public.h`; Copyright (C) 1999-2005 Id Software,
//! Inc., GPL-2.0-or-later). Each import enum decodes the raw trap word back to a
//! symbolic code; unknown words decode to `None` so the dispatcher can treat
//! them as extensions.

/// Server-game engine imports (`G_*`, `BOTLIB_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum QvmGameImport {
    /// Print a message.
    GPrint = 0,
    /// Abort with an error.
    GError = 1,
    /// Milliseconds since server start.
    GMilliseconds = 2,
    /// Register a console variable.
    GCvarRegister = 3,
    /// Update a console variable.
    GCvarUpdate = 4,
    /// Set a console variable.
    GCvarSet = 5,
    /// Read an integer console variable.
    GCvarVariableIntegerValue = 6,
    /// Read a string console variable.
    GCvarVariableStringBuffer = 7,
    /// Command argument count.
    GArgc = 8,
    /// Command argument.
    GArgv = 9,
    /// Open a file.
    GFOpenFile = 10,
    /// Read a file.
    GFRead = 11,
    /// Write a file.
    GFWrite = 12,
    /// Close a file.
    GFCloseFile = 13,
    /// Send a console command.
    GSendConsoleCommand = 14,
    /// Locate shared game data.
    GLocateGameData = 15,
    /// Drop a client.
    GDropClient = 16,
    /// Send a server command.
    GSendServerCommand = 17,
    /// Set a config string.
    GSetConfigstring = 18,
    /// Get a config string.
    GGetConfigstring = 19,
    /// Get user info.
    GGetUserinfo = 20,
    /// Set user info.
    GSetUserinfo = 21,
    /// Get server info.
    GGetServerinfo = 22,
    /// Set a brush model.
    GSetBrushModel = 23,
    /// Trace a ray.
    GTrace = 24,
    /// Point contents.
    GPointContents = 25,
    /// Potentially-visible-set test.
    GInPvs = 26,
    /// PVS test ignoring portals.
    GInPvsIgnorePortals = 27,
    /// Adjust an area portal state.
    GAdjustAreaPortalState = 28,
    /// Area connectivity test.
    GAreasConnected = 29,
    /// Link an entity.
    GLinkentity = 30,
    /// Unlink an entity.
    GUnlinkentity = 31,
    /// Entities in a box.
    GEntitiesInBox = 32,
    /// Entity contact test.
    GEntityContact = 33,
    /// Allocate a bot client.
    GBotAllocateClient = 34,
    /// Free a bot client.
    GBotFreeClient = 35,
    /// Get a user command.
    GGetUsercmd = 36,
    /// Get an entity token.
    GGetEntityToken = 37,
    /// List files.
    GFGetfilelist = 38,
    /// Create a debug polygon.
    GDebugPolygonCreate = 39,
    /// Delete a debug polygon.
    GDebugPolygonDelete = 40,
    /// Real time.
    GRealTime = 41,
    /// Snap a vector to integer coordinates.
    GSnapvector = 42,
    /// Capsule trace.
    GTracecapsule = 43,
    /// Capsule entity contact.
    GEntityContactcapsule = 44,
    /// Seek a file.
    GFSeek = 45,
    /// Set up the bot library.
    BotlibSetup = 200,
    /// Shut down the bot library.
    BotlibShutdown = 201,
    /// Set a bot library variable.
    BotlibLibvarSet = 202,
    /// Get a bot library variable.
    BotlibLibvarGet = 203,
    /// Add a global precompiler define.
    BotlibPcAddGlobalDefine = 204,
    /// Start a bot frame.
    BotlibStartFrame = 205,
    /// Load a bot map.
    BotlibLoadMap = 206,
    /// Update a bot entity.
    BotlibUpdatentity = 207,
    /// Bot library test hook.
    BotlibTest = 208,
    /// Get a snapshot entity.
    BotlibGetSnapshotEntity = 209,
    /// Get a console message.
    BotlibGetConsoleMessage = 210,
    /// Run a bot user command.
    BotlibUserCommand = 211,
    /// Enable an AAS routing area.
    BotlibAasEnableRoutingArea = 300,
    /// AAS bounding-box areas.
    BotlibAasBboxAreas = 301,
    /// AAS area info.
    BotlibAasAreaInfo = 302,
    /// AAS entity info.
    BotlibAasEntityInfo = 303,
    /// Whether AAS is initialized.
    BotlibAasInitialized = 304,
    /// AAS presence bounding box.
    BotlibAasPresenceTypeBoundingBox = 305,
    /// AAS time.
    BotlibAasTime = 306,
    /// AAS point area number.
    BotlibAasPointAreaNum = 307,
    /// AAS trace areas.
    BotlibAasTraceAreas = 308,
    /// AAS point contents.
    BotlibAasPointContents = 309,
    /// Next AAS BSP entity.
    BotlibAasNextBspEntity = 310,
    /// AAS BSP epair string value.
    BotlibAasValueForBspEpairKey = 311,
    /// AAS BSP epair vector value.
    BotlibAasVectorForBspEpairKey = 312,
    /// AAS BSP epair float value.
    BotlibAasFloatForBspEpairKey = 313,
    /// AAS BSP epair integer value.
    BotlibAasIntForBspEpairKey = 314,
    /// AAS area reachability.
    BotlibAasAreaReachability = 315,
    /// AAS travel time to a goal area.
    BotlibAasAreaTravelTimeToGoalArea = 316,
    /// AAS swimming query.
    BotlibAasSwimming = 317,
    /// Predict client movement.
    BotlibAasPredictClientMovement = 318,
    /// Bot chat.
    BotlibEaSay = 400,
    /// Bot team chat.
    BotlibEaSayTeam = 401,
    /// Bot console command.
    BotlibEaCommand = 402,
    /// Bot action.
    BotlibEaAction = 403,
    /// Bot gesture.
    BotlibEaGesture = 404,
    /// Bot talk.
    BotlibEaTalk = 405,
    /// Bot attack.
    BotlibEaAttack = 406,
    /// Bot use.
    BotlibEaUse = 407,
    /// Bot respawn.
    BotlibEaRespawn = 408,
    /// Bot crouch.
    BotlibEaCrouch = 409,
    /// Bot move up.
    BotlibEaMoveUp = 410,
    /// Bot move down.
    BotlibEaMoveDown = 411,
    /// Bot move forward.
    BotlibEaMoveForward = 412,
    /// Bot move back.
    BotlibEaMoveBack = 413,
    /// Bot move left.
    BotlibEaMoveLeft = 414,
    /// Bot move right.
    BotlibEaMoveRight = 415,
    /// Bot weapon selection.
    BotlibEaSelectWeapon = 416,
    /// Bot jump.
    BotlibEaJump = 417,
    /// Bot delayed jump.
    BotlibEaDelayedJump = 418,
    /// Bot move.
    BotlibEaMove = 419,
    /// Bot view angles.
    BotlibEaView = 420,
    /// End regular bot input.
    BotlibEaEndRegular = 421,
    /// Get bot input.
    BotlibEaGetInput = 422,
    /// Reset bot input.
    BotlibEaResetInput = 423,
    /// Load a bot character.
    BotlibAiLoadCharacter = 500,
    /// Free a bot character.
    BotlibAiFreeCharacter = 501,
    /// Float characteristic.
    BotlibAiCharacteristicFloat = 502,
    /// Bounded float characteristic.
    BotlibAiCharacteristicBfloat = 503,
    /// Integer characteristic.
    BotlibAiCharacteristicInteger = 504,
    /// Bounded integer characteristic.
    BotlibAiCharacteristicBinteger = 505,
    /// String characteristic.
    BotlibAiCharacteristicString = 506,
    /// Allocate chat state.
    BotlibAiAllocChatState = 507,
    /// Free chat state.
    BotlibAiFreeChatState = 508,
    /// Queue a console message.
    BotlibAiQueueConsoleMessage = 509,
    /// Remove a console message.
    BotlibAiRemoveConsoleMessage = 510,
    /// Next console message.
    BotlibAiNextConsoleMessage = 511,
    /// Console message count.
    BotlibAiNumConsoleMessage = 512,
    /// Initial chat.
    BotlibAiInitialChat = 513,
    /// Reply chat.
    BotlibAiReplyChat = 514,
    /// Chat length.
    BotlibAiChatLength = 515,
    /// Enter chat.
    BotlibAiEnterChat = 516,
    /// String containment test.
    BotlibAiStringContains = 517,
    /// Find a chat match.
    BotlibAiFindMatch = 518,
    /// Match a chat variable.
    BotlibAiMatchVariable = 519,
    /// Unify white space.
    BotlibAiUnifyWhiteSpaces = 520,
    /// Replace synonyms.
    BotlibAiReplaceSynonyms = 521,
    /// Load a chat file.
    BotlibAiLoadChatFile = 522,
    /// Set chat gender.
    BotlibAiSetChatGender = 523,
    /// Set chat name.
    BotlibAiSetChatName = 524,
    /// Reset goal state.
    BotlibAiResetGoalState = 525,
    /// Reset avoid goals.
    BotlibAiResetAvoidGoals = 526,
    /// Push a goal.
    BotlibAiPushGoal = 527,
    /// Pop a goal.
    BotlibAiPopGoal = 528,
    /// Empty the goal stack.
    BotlibAiEmptyGoalStack = 529,
    /// Dump avoid goals.
    BotlibAiDumpAvoidGoals = 530,
    /// Dump the goal stack.
    BotlibAiDumpGoalStack = 531,
    /// Goal name.
    BotlibAiGoalName = 532,
    /// Top goal.
    BotlibAiGetTopGoal = 533,
    /// Second goal.
    BotlibAiGetSecondGoal = 534,
    /// Choose a long-term goal item.
    BotlibAiChooseLtgItem = 535,
    /// Choose a nearby goal item.
    BotlibAiChooseNbgItem = 536,
    /// Touching-goal test.
    BotlibAiTouchingGoal = 537,
    /// Item visible-but-not-visible test.
    BotlibAiItemGoalInVisButNotVisible = 538,
    /// Level item goal.
    BotlibAiGetLevelItemGoal = 539,
    /// Avoid-goal time.
    BotlibAiAvoidGoalTime = 540,
    /// Initialize level items.
    BotlibAiInitLevelItems = 541,
    /// Update entity items.
    BotlibAiUpdateEntityItems = 542,
    /// Load item weights.
    BotlibAiLoadItemWeights = 543,
    /// Free item weights.
    BotlibAiFreeItemWeights = 544,
    /// Save goal fuzzy logic.
    BotlibAiSaveGoalFuzzyLogic = 545,
    /// Allocate goal state.
    BotlibAiAllocGoalState = 546,
    /// Free goal state.
    BotlibAiFreeGoalState = 547,
    /// Reset move state.
    BotlibAiResetMoveState = 548,
    /// Move to a goal.
    BotlibAiMoveToGoal = 549,
    /// Move in a direction.
    BotlibAiMoveInDirection = 550,
    /// Reset avoid reach.
    BotlibAiResetAvoidReach = 551,
    /// Reset last avoid reach.
    BotlibAiResetLastAvoidReach = 552,
    /// Reachability area.
    BotlibAiReachabilityArea = 553,
    /// Movement view target.
    BotlibAiMovementViewTarget = 554,
    /// Allocate move state.
    BotlibAiAllocMoveState = 555,
    /// Free move state.
    BotlibAiFreeMoveState = 556,
    /// Initialize move state.
    BotlibAiInitMoveState = 557,
    /// Choose the best fight weapon.
    BotlibAiChooseBestFightWeapon = 558,
    /// Get weapon info.
    BotlibAiGetWeaponInfo = 559,
    /// Load weapon weights.
    BotlibAiLoadWeaponWeights = 560,
    /// Allocate weapon state.
    BotlibAiAllocWeaponState = 561,
    /// Free weapon state.
    BotlibAiFreeWeaponState = 562,
    /// Reset weapon state.
    BotlibAiResetWeaponState = 563,
    /// Genetic parent/child selection.
    BotlibAiGeneticParentsAndChildSelection = 564,
    /// Interbreed goal fuzzy logic.
    BotlibAiInterbreedGoalFuzzyLogic = 565,
    /// Mutate goal fuzzy logic.
    BotlibAiMutateGoalFuzzyLogic = 566,
    /// Next camp-spot goal.
    BotlibAiGetNextCampSpotGoal = 567,
    /// Map location goal.
    BotlibAiGetMapLocationGoal = 568,
    /// Initial chat count.
    BotlibAiNumInitialChats = 569,
    /// Get a chat message.
    BotlibAiGetChatMessage = 570,
    /// Remove from avoid goals.
    BotlibAiRemoveFromAvoidGoals = 571,
    /// Predict a visible position.
    BotlibAiPredictVisiblePosition = 572,
    /// Set avoid-goal time.
    BotlibAiSetAvoidGoalTime = 573,
    /// Add an avoid spot.
    BotlibAiAddAvoidSpot = 574,
    /// Alternative route goal.
    BotlibAasAlternativeRouteGoal = 575,
    /// Predict a route.
    BotlibAasPredictRoute = 576,
    /// Point reachability area index.
    BotlibAasPointReachabilityAreaIndex = 577,
    /// Load a precompiler source.
    BotlibPcLoadSource = 578,
    /// Free a precompiler source.
    BotlibPcFreeSource = 579,
    /// Read a precompiler token.
    BotlibPcReadToken = 580,
    /// Precompiler source file and line.
    BotlibPcSourceFileAndLine = 581,
}

/// Decode a server-game trap word.
#[must_use]
pub fn decode_qvm_game_import(word: i32) -> Option<QvmGameImport> {
    use QvmGameImport as G;
    match word {
        0 => Some(G::GPrint),
        1 => Some(G::GError),
        2 => Some(G::GMilliseconds),
        3 => Some(G::GCvarRegister),
        4 => Some(G::GCvarUpdate),
        5 => Some(G::GCvarSet),
        6 => Some(G::GCvarVariableIntegerValue),
        7 => Some(G::GCvarVariableStringBuffer),
        8 => Some(G::GArgc),
        9 => Some(G::GArgv),
        10 => Some(G::GFOpenFile),
        11 => Some(G::GFRead),
        12 => Some(G::GFWrite),
        13 => Some(G::GFCloseFile),
        14 => Some(G::GSendConsoleCommand),
        15 => Some(G::GLocateGameData),
        16 => Some(G::GDropClient),
        17 => Some(G::GSendServerCommand),
        18 => Some(G::GSetConfigstring),
        19 => Some(G::GGetConfigstring),
        20 => Some(G::GGetUserinfo),
        21 => Some(G::GSetUserinfo),
        22 => Some(G::GGetServerinfo),
        23 => Some(G::GSetBrushModel),
        24 => Some(G::GTrace),
        25 => Some(G::GPointContents),
        26 => Some(G::GInPvs),
        27 => Some(G::GInPvsIgnorePortals),
        28 => Some(G::GAdjustAreaPortalState),
        29 => Some(G::GAreasConnected),
        30 => Some(G::GLinkentity),
        31 => Some(G::GUnlinkentity),
        32 => Some(G::GEntitiesInBox),
        33 => Some(G::GEntityContact),
        34 => Some(G::GBotAllocateClient),
        35 => Some(G::GBotFreeClient),
        36 => Some(G::GGetUsercmd),
        37 => Some(G::GGetEntityToken),
        38 => Some(G::GFGetfilelist),
        39 => Some(G::GDebugPolygonCreate),
        40 => Some(G::GDebugPolygonDelete),
        41 => Some(G::GRealTime),
        42 => Some(G::GSnapvector),
        43 => Some(G::GTracecapsule),
        44 => Some(G::GEntityContactcapsule),
        45 => Some(G::GFSeek),
        200 => Some(G::BotlibSetup),
        201 => Some(G::BotlibShutdown),
        202 => Some(G::BotlibLibvarSet),
        203 => Some(G::BotlibLibvarGet),
        204 => Some(G::BotlibPcAddGlobalDefine),
        205 => Some(G::BotlibStartFrame),
        206 => Some(G::BotlibLoadMap),
        207 => Some(G::BotlibUpdatentity),
        208 => Some(G::BotlibTest),
        209 => Some(G::BotlibGetSnapshotEntity),
        210 => Some(G::BotlibGetConsoleMessage),
        211 => Some(G::BotlibUserCommand),
        300 => Some(G::BotlibAasEnableRoutingArea),
        301 => Some(G::BotlibAasBboxAreas),
        302 => Some(G::BotlibAasAreaInfo),
        303 => Some(G::BotlibAasEntityInfo),
        304 => Some(G::BotlibAasInitialized),
        305 => Some(G::BotlibAasPresenceTypeBoundingBox),
        306 => Some(G::BotlibAasTime),
        307 => Some(G::BotlibAasPointAreaNum),
        308 => Some(G::BotlibAasTraceAreas),
        309 => Some(G::BotlibAasPointContents),
        310 => Some(G::BotlibAasNextBspEntity),
        311 => Some(G::BotlibAasValueForBspEpairKey),
        312 => Some(G::BotlibAasVectorForBspEpairKey),
        313 => Some(G::BotlibAasFloatForBspEpairKey),
        314 => Some(G::BotlibAasIntForBspEpairKey),
        315 => Some(G::BotlibAasAreaReachability),
        316 => Some(G::BotlibAasAreaTravelTimeToGoalArea),
        317 => Some(G::BotlibAasSwimming),
        318 => Some(G::BotlibAasPredictClientMovement),
        400 => Some(G::BotlibEaSay),
        401 => Some(G::BotlibEaSayTeam),
        402 => Some(G::BotlibEaCommand),
        403 => Some(G::BotlibEaAction),
        404 => Some(G::BotlibEaGesture),
        405 => Some(G::BotlibEaTalk),
        406 => Some(G::BotlibEaAttack),
        407 => Some(G::BotlibEaUse),
        408 => Some(G::BotlibEaRespawn),
        409 => Some(G::BotlibEaCrouch),
        410 => Some(G::BotlibEaMoveUp),
        411 => Some(G::BotlibEaMoveDown),
        412 => Some(G::BotlibEaMoveForward),
        413 => Some(G::BotlibEaMoveBack),
        414 => Some(G::BotlibEaMoveLeft),
        415 => Some(G::BotlibEaMoveRight),
        416 => Some(G::BotlibEaSelectWeapon),
        417 => Some(G::BotlibEaJump),
        418 => Some(G::BotlibEaDelayedJump),
        419 => Some(G::BotlibEaMove),
        420 => Some(G::BotlibEaView),
        421 => Some(G::BotlibEaEndRegular),
        422 => Some(G::BotlibEaGetInput),
        423 => Some(G::BotlibEaResetInput),
        500 => Some(G::BotlibAiLoadCharacter),
        501 => Some(G::BotlibAiFreeCharacter),
        502 => Some(G::BotlibAiCharacteristicFloat),
        503 => Some(G::BotlibAiCharacteristicBfloat),
        504 => Some(G::BotlibAiCharacteristicInteger),
        505 => Some(G::BotlibAiCharacteristicBinteger),
        506 => Some(G::BotlibAiCharacteristicString),
        507 => Some(G::BotlibAiAllocChatState),
        508 => Some(G::BotlibAiFreeChatState),
        509 => Some(G::BotlibAiQueueConsoleMessage),
        510 => Some(G::BotlibAiRemoveConsoleMessage),
        511 => Some(G::BotlibAiNextConsoleMessage),
        512 => Some(G::BotlibAiNumConsoleMessage),
        513 => Some(G::BotlibAiInitialChat),
        514 => Some(G::BotlibAiReplyChat),
        515 => Some(G::BotlibAiChatLength),
        516 => Some(G::BotlibAiEnterChat),
        517 => Some(G::BotlibAiStringContains),
        518 => Some(G::BotlibAiFindMatch),
        519 => Some(G::BotlibAiMatchVariable),
        520 => Some(G::BotlibAiUnifyWhiteSpaces),
        521 => Some(G::BotlibAiReplaceSynonyms),
        522 => Some(G::BotlibAiLoadChatFile),
        523 => Some(G::BotlibAiSetChatGender),
        524 => Some(G::BotlibAiSetChatName),
        525 => Some(G::BotlibAiResetGoalState),
        526 => Some(G::BotlibAiResetAvoidGoals),
        527 => Some(G::BotlibAiPushGoal),
        528 => Some(G::BotlibAiPopGoal),
        529 => Some(G::BotlibAiEmptyGoalStack),
        530 => Some(G::BotlibAiDumpAvoidGoals),
        531 => Some(G::BotlibAiDumpGoalStack),
        532 => Some(G::BotlibAiGoalName),
        533 => Some(G::BotlibAiGetTopGoal),
        534 => Some(G::BotlibAiGetSecondGoal),
        535 => Some(G::BotlibAiChooseLtgItem),
        536 => Some(G::BotlibAiChooseNbgItem),
        537 => Some(G::BotlibAiTouchingGoal),
        538 => Some(G::BotlibAiItemGoalInVisButNotVisible),
        539 => Some(G::BotlibAiGetLevelItemGoal),
        540 => Some(G::BotlibAiAvoidGoalTime),
        541 => Some(G::BotlibAiInitLevelItems),
        542 => Some(G::BotlibAiUpdateEntityItems),
        543 => Some(G::BotlibAiLoadItemWeights),
        544 => Some(G::BotlibAiFreeItemWeights),
        545 => Some(G::BotlibAiSaveGoalFuzzyLogic),
        546 => Some(G::BotlibAiAllocGoalState),
        547 => Some(G::BotlibAiFreeGoalState),
        548 => Some(G::BotlibAiResetMoveState),
        549 => Some(G::BotlibAiMoveToGoal),
        550 => Some(G::BotlibAiMoveInDirection),
        551 => Some(G::BotlibAiResetAvoidReach),
        552 => Some(G::BotlibAiResetLastAvoidReach),
        553 => Some(G::BotlibAiReachabilityArea),
        554 => Some(G::BotlibAiMovementViewTarget),
        555 => Some(G::BotlibAiAllocMoveState),
        556 => Some(G::BotlibAiFreeMoveState),
        557 => Some(G::BotlibAiInitMoveState),
        558 => Some(G::BotlibAiChooseBestFightWeapon),
        559 => Some(G::BotlibAiGetWeaponInfo),
        560 => Some(G::BotlibAiLoadWeaponWeights),
        561 => Some(G::BotlibAiAllocWeaponState),
        562 => Some(G::BotlibAiFreeWeaponState),
        563 => Some(G::BotlibAiResetWeaponState),
        564 => Some(G::BotlibAiGeneticParentsAndChildSelection),
        565 => Some(G::BotlibAiInterbreedGoalFuzzyLogic),
        566 => Some(G::BotlibAiMutateGoalFuzzyLogic),
        567 => Some(G::BotlibAiGetNextCampSpotGoal),
        568 => Some(G::BotlibAiGetMapLocationGoal),
        569 => Some(G::BotlibAiNumInitialChats),
        570 => Some(G::BotlibAiGetChatMessage),
        571 => Some(G::BotlibAiRemoveFromAvoidGoals),
        572 => Some(G::BotlibAiPredictVisiblePosition),
        573 => Some(G::BotlibAiSetAvoidGoalTime),
        574 => Some(G::BotlibAiAddAvoidSpot),
        575 => Some(G::BotlibAasAlternativeRouteGoal),
        576 => Some(G::BotlibAasPredictRoute),
        577 => Some(G::BotlibAasPointReachabilityAreaIndex),
        578 => Some(G::BotlibPcLoadSource),
        579 => Some(G::BotlibPcFreeSource),
        580 => Some(G::BotlibPcReadToken),
        581 => Some(G::BotlibPcSourceFileAndLine),
        _ => None,
    }
}

/// Server-game exports called by the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum QvmGameExport {
    /// Initialize the game.
    GameInit = 0,
    /// Shut down the game.
    GameShutdown = 1,
    /// A client connects.
    GameClientConnect = 2,
    /// A client begins.
    GameClientBegin = 3,
    /// Client user info changed.
    GameClientUserinfoChanged = 4,
    /// A client disconnects.
    GameClientDisconnect = 5,
    /// A client command.
    GameClientCommand = 6,
    /// A client think.
    GameClientThink = 7,
    /// Run a server frame.
    GameRunFrame = 8,
    /// A console command.
    GameConsoleCommand = 9,
    /// Start a bot AI frame.
    BotaiStartFrame = 10,
}

/// Client-game engine imports (`CG_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum QvmCgameImport {
    /// Print a message.
    CgPrint = 0,
    /// Abort with an error.
    CgError = 1,
    /// Milliseconds since client start.
    CgMilliseconds = 2,
    /// Register a console variable.
    CgCvarRegister = 3,
    /// Update a console variable.
    CgCvarUpdate = 4,
    /// Set a console variable.
    CgCvarSet = 5,
    /// Read a string console variable.
    CgCvarVariablestringbuffer = 6,
    /// Command argument count.
    CgArgc = 7,
    /// Command argument.
    CgArgv = 8,
    /// Full command arguments.
    CgArgs = 9,
    /// Open a file.
    CgFsFopenfile = 10,
    /// Read a file.
    CgFsRead = 11,
    /// Write a file.
    CgFsWrite = 12,
    /// Close a file.
    CgFsFclosefile = 13,
    /// Send a console command.
    CgSendconsolecommand = 14,
    /// Add a console command.
    CgAddcommand = 15,
    /// Send a client command.
    CgSendclientcommand = 16,
    /// Update the screen.
    CgUpdatescreen = 17,
    /// Load a collision map.
    CgCmLoadmap = 18,
    /// Inline model count.
    CgCmNuminlinemodels = 19,
    /// Inline model handle.
    CgCmInlinemodel = 20,
    /// Load a collision model.
    CgCmLoadmodel = 21,
    /// Temporary box model.
    CgCmTempboxmodel = 22,
    /// Point contents.
    CgCmPointcontents = 23,
    /// Transformed point contents.
    CgCmTransformedpointcontents = 24,
    /// Box trace.
    CgCmBoxtrace = 25,
    /// Transformed box trace.
    CgCmTransformedboxtrace = 26,
    /// Mark fragments.
    CgCmMarkfragments = 27,
    /// Start a sound.
    CgSStartsound = 28,
    /// Start a local sound.
    CgSStartlocalsound = 29,
    /// Clear looping sounds.
    CgSClearloopingsounds = 30,
    /// Add a looping sound.
    CgSAddloopingsound = 31,
    /// Update an entity position.
    CgSUpdateentityposition = 32,
    /// Respatialize sounds.
    CgSRespatialize = 33,
    /// Register a sound.
    CgSRegistersound = 34,
    /// Start the background track.
    CgSStartbackgroundtrack = 35,
    /// Load the world map.
    CgRLoadworldmap = 36,
    /// Register a model.
    CgRRegistermodel = 37,
    /// Register a skin.
    CgRRegisterskin = 38,
    /// Register a shader.
    CgRRegistershader = 39,
    /// Clear the scene.
    CgRClearscene = 40,
    /// Add a reference entity.
    CgRAddrefentitytoscene = 41,
    /// Add a polygon.
    CgRAddpolytoscene = 42,
    /// Add a light.
    CgRAddlighttoscene = 43,
    /// Render the scene.
    CgRenderscene = 44,
    /// Set the draw color.
    CgRSetcolor = 45,
    /// Draw a stretched picture.
    CgRDrawstretchpic = 46,
    /// Model bounds.
    CgRModelbounds = 47,
    /// Lerp a tag.
    CgRLerptag = 48,
    /// GL configuration.
    CgGetglconfig = 49,
    /// Game state.
    CgGetgamestate = 50,
    /// Current snapshot number.
    CgGetcurrentsnapshotnumber = 51,
    /// Get a snapshot.
    CgGetsnapshot = 52,
    /// Get a server command.
    CgGetservercommand = 53,
    /// Current command number.
    CgGetcurrentcmdnumber = 54,
    /// Get a user command.
    CgGetusercmd = 55,
    /// Set user command value.
    CgSetusercmdvalue = 56,
    /// Register a no-mip shader.
    CgRRegistershadernomip = 57,
    /// Remaining memory.
    CgMemoryRemaining = 58,
    /// Register a font.
    CgRRegisterfont = 59,
    /// Key down test.
    CgKeyIsdown = 60,
    /// Get the key catcher.
    CgKeyGetcatcher = 61,
    /// Set the key catcher.
    CgKeySetcatcher = 62,
    /// Get a key.
    CgKeyGetkey = 63,
    /// Add a global precompiler define.
    CgPcAddGlobalDefine = 64,
    /// Load a precompiler source.
    CgPcLoadSource = 65,
    /// Free a precompiler source.
    CgPcFreeSource = 66,
    /// Read a precompiler token.
    CgPcReadToken = 67,
    /// Precompiler source file and line.
    CgPcSourceFileAndLine = 68,
    /// Stop the background track.
    CgSStopbackgroundtrack = 69,
    /// Real time.
    CgRealTime = 70,
    /// Snap a vector.
    CgSnapvector = 71,
    /// Remove a console command.
    CgRemovecommand = 72,
    /// Light for a point.
    CgRLightforpoint = 73,
    /// Play a cinematic.
    CgCinPlaycinematic = 74,
    /// Stop a cinematic.
    CgCinStopcinematic = 75,
    /// Run a cinematic.
    CgCinRuncinematic = 76,
    /// Draw a cinematic.
    CgCinDrawcinematic = 77,
    /// Set cinematic extents.
    CgCinSetextents = 78,
    /// Remap a shader.
    CgRRemapShader = 79,
    /// Add a real looping sound.
    CgSAddrealloopingsound = 80,
    /// Stop a looping sound.
    CgSStoploopingsound = 81,
    /// Temporary capsule model.
    CgCmTempcapsulemodel = 82,
    /// Capsule trace.
    CgCmCapsuletrace = 83,
    /// Transformed capsule trace.
    CgCmTransformedcapsuletrace = 84,
    /// Add an additive light.
    CgRAddadditivelighttoscene = 85,
    /// Get an entity token.
    CgGetEntityToken = 86,
    /// Add polygons.
    CgRAddpolystoscene = 87,
    /// In-PVS test.
    CgRInpvs = 88,
    /// Seek a file.
    CgFsSeek = 89,
    /// Memory set.
    CgMemset = 100,
    /// Memory copy.
    CgMemcpy = 101,
    /// Bounded string copy.
    CgStrncpy = 102,
    /// Sine.
    CgSin = 103,
    /// Cosine.
    CgCos = 104,
    /// Arc tangent.
    CgAtan2 = 105,
    /// Square root.
    CgSqrt = 106,
    /// Floor.
    CgFloor = 107,
    /// Ceiling.
    CgCeil = 108,
    /// Test integer print.
    CgTestprintint = 109,
    /// Test float print.
    CgTestprintfloat = 110,
    /// Arc cosine.
    CgAcos = 111,
}

/// Decode a client-game trap word.
#[must_use]
pub fn decode_qvm_cgame_import(word: i32) -> Option<QvmCgameImport> {
    use QvmCgameImport as C;
    match word {
        0 => Some(C::CgPrint),
        1 => Some(C::CgError),
        2 => Some(C::CgMilliseconds),
        3 => Some(C::CgCvarRegister),
        4 => Some(C::CgCvarUpdate),
        5 => Some(C::CgCvarSet),
        6 => Some(C::CgCvarVariablestringbuffer),
        7 => Some(C::CgArgc),
        8 => Some(C::CgArgv),
        9 => Some(C::CgArgs),
        10 => Some(C::CgFsFopenfile),
        11 => Some(C::CgFsRead),
        12 => Some(C::CgFsWrite),
        13 => Some(C::CgFsFclosefile),
        14 => Some(C::CgSendconsolecommand),
        15 => Some(C::CgAddcommand),
        16 => Some(C::CgSendclientcommand),
        17 => Some(C::CgUpdatescreen),
        18 => Some(C::CgCmLoadmap),
        19 => Some(C::CgCmNuminlinemodels),
        20 => Some(C::CgCmInlinemodel),
        21 => Some(C::CgCmLoadmodel),
        22 => Some(C::CgCmTempboxmodel),
        23 => Some(C::CgCmPointcontents),
        24 => Some(C::CgCmTransformedpointcontents),
        25 => Some(C::CgCmBoxtrace),
        26 => Some(C::CgCmTransformedboxtrace),
        27 => Some(C::CgCmMarkfragments),
        28 => Some(C::CgSStartsound),
        29 => Some(C::CgSStartlocalsound),
        30 => Some(C::CgSClearloopingsounds),
        31 => Some(C::CgSAddloopingsound),
        32 => Some(C::CgSUpdateentityposition),
        33 => Some(C::CgSRespatialize),
        34 => Some(C::CgSRegistersound),
        35 => Some(C::CgSStartbackgroundtrack),
        36 => Some(C::CgRLoadworldmap),
        37 => Some(C::CgRRegistermodel),
        38 => Some(C::CgRRegisterskin),
        39 => Some(C::CgRRegistershader),
        40 => Some(C::CgRClearscene),
        41 => Some(C::CgRAddrefentitytoscene),
        42 => Some(C::CgRAddpolytoscene),
        43 => Some(C::CgRAddlighttoscene),
        44 => Some(C::CgRenderscene),
        45 => Some(C::CgRSetcolor),
        46 => Some(C::CgRDrawstretchpic),
        47 => Some(C::CgRModelbounds),
        48 => Some(C::CgRLerptag),
        49 => Some(C::CgGetglconfig),
        50 => Some(C::CgGetgamestate),
        51 => Some(C::CgGetcurrentsnapshotnumber),
        52 => Some(C::CgGetsnapshot),
        53 => Some(C::CgGetservercommand),
        54 => Some(C::CgGetcurrentcmdnumber),
        55 => Some(C::CgGetusercmd),
        56 => Some(C::CgSetusercmdvalue),
        57 => Some(C::CgRRegistershadernomip),
        58 => Some(C::CgMemoryRemaining),
        59 => Some(C::CgRRegisterfont),
        60 => Some(C::CgKeyIsdown),
        61 => Some(C::CgKeyGetcatcher),
        62 => Some(C::CgKeySetcatcher),
        63 => Some(C::CgKeyGetkey),
        64 => Some(C::CgPcAddGlobalDefine),
        65 => Some(C::CgPcLoadSource),
        66 => Some(C::CgPcFreeSource),
        67 => Some(C::CgPcReadToken),
        68 => Some(C::CgPcSourceFileAndLine),
        69 => Some(C::CgSStopbackgroundtrack),
        70 => Some(C::CgRealTime),
        71 => Some(C::CgSnapvector),
        72 => Some(C::CgRemovecommand),
        73 => Some(C::CgRLightforpoint),
        74 => Some(C::CgCinPlaycinematic),
        75 => Some(C::CgCinStopcinematic),
        76 => Some(C::CgCinRuncinematic),
        77 => Some(C::CgCinDrawcinematic),
        78 => Some(C::CgCinSetextents),
        79 => Some(C::CgRRemapShader),
        80 => Some(C::CgSAddrealloopingsound),
        81 => Some(C::CgSStoploopingsound),
        82 => Some(C::CgCmTempcapsulemodel),
        83 => Some(C::CgCmCapsuletrace),
        84 => Some(C::CgCmTransformedcapsuletrace),
        85 => Some(C::CgRAddadditivelighttoscene),
        86 => Some(C::CgGetEntityToken),
        87 => Some(C::CgRAddpolystoscene),
        88 => Some(C::CgRInpvs),
        89 => Some(C::CgFsSeek),
        100 => Some(C::CgMemset),
        101 => Some(C::CgMemcpy),
        102 => Some(C::CgStrncpy),
        103 => Some(C::CgSin),
        104 => Some(C::CgCos),
        105 => Some(C::CgAtan2),
        106 => Some(C::CgSqrt),
        107 => Some(C::CgFloor),
        108 => Some(C::CgCeil),
        109 => Some(C::CgTestprintint),
        110 => Some(C::CgTestprintfloat),
        111 => Some(C::CgAcos),
        _ => None,
    }
}

/// Client-game exports called by the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum QvmCgameExport {
    /// Initialize the client game.
    CgInit = 0,
    /// Shut down the client game.
    CgShutdown = 1,
    /// A console command.
    CgConsoleCommand = 2,
    /// Draw the active frame.
    CgDrawActiveFrame = 3,
    /// Crosshair player.
    CgCrosshairPlayer = 4,
    /// Last attacker.
    CgLastAttacker = 5,
    /// A key event.
    CgKeyEvent = 6,
    /// A mouse event.
    CgMouseEvent = 7,
    /// Event handling.
    CgEventHandling = 8,
}

/// UI engine imports (`UI_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum QvmUiImport {
    /// Abort with an error.
    UiError = 0,
    /// Print a message.
    UiPrint = 1,
    /// Milliseconds since client start.
    UiMilliseconds = 2,
    /// Set a console variable.
    UiCvarSet = 3,
    /// Read a float console variable.
    UiCvarVariablevalue = 4,
    /// Read a string console variable.
    UiCvarVariablestringbuffer = 5,
    /// Set a float console variable.
    UiCvarSetvalue = 6,
    /// Reset a console variable.
    UiCvarReset = 7,
    /// Create a console variable.
    UiCvarCreate = 8,
    /// Read an info string.
    UiCvarInfostringbuffer = 9,
    /// Command argument count.
    UiArgc = 10,
    /// Command argument.
    UiArgv = 11,
    /// Execute console text.
    UiCmdExecutetext = 12,
    /// Open a file.
    UiFsFopenfile = 13,
    /// Read a file.
    UiFsRead = 14,
    /// Write a file.
    UiFsWrite = 15,
    /// Close a file.
    UiFsFclosefile = 16,
    /// List files.
    UiFsGetfilelist = 17,
    /// Register a model.
    UiRRegistermodel = 18,
    /// Register a skin.
    UiRRegisterskin = 19,
    /// Register a no-mip shader.
    UiRRegistershadernomip = 20,
    /// Clear the scene.
    UiRClearscene = 21,
    /// Add a reference entity.
    UiRAddrefentitytoscene = 22,
    /// Add a polygon.
    UiRAddpolytoscene = 23,
    /// Add a light.
    UiRAddlighttoscene = 24,
    /// Render the scene.
    UiRenderscene = 25,
    /// Set the draw color.
    UiRSetcolor = 26,
    /// Draw a stretched picture.
    UiRDrawstretchpic = 27,
    /// Update the screen.
    UiUpdatescreen = 28,
    /// Lerp a tag.
    UiCmLerptag = 29,
    /// Load a collision model.
    UiCmLoadmodel = 30,
    /// Register a sound.
    UiSRegistersound = 31,
    /// Start a local sound.
    UiSStartlocalsound = 32,
    /// Key number to string.
    UiKeyKeynumtostringbuf = 33,
    /// Get a key binding.
    UiKeyGetbindingbuf = 34,
    /// Set a key binding.
    UiKeySetbinding = 35,
    /// Key down test.
    UiKeyIsdown = 36,
    /// Get overstrike mode.
    UiKeyGetoverstrikemode = 37,
    /// Set overstrike mode.
    UiKeySetoverstrikemode = 38,
    /// Clear key states.
    UiKeyClearstates = 39,
    /// Get the key catcher.
    UiKeyGetcatcher = 40,
    /// Set the key catcher.
    UiKeySetcatcher = 41,
    /// Get clipboard data.
    UiGetclipboarddata = 42,
    /// GL configuration.
    UiGetglconfig = 43,
    /// Client state.
    UiGetclientstate = 44,
    /// Get a config string.
    UiGetconfigstring = 45,
    /// LAN ping queue count.
    UiLanGetpingqueuecount = 46,
    /// Clear a LAN ping.
    UiLanClearping = 47,
    /// Get a LAN ping.
    UiLanGetping = 48,
    /// Get LAN ping info.
    UiLanGetpinginfo = 49,
    /// Register a console variable.
    UiCvarRegister = 50,
    /// Update a console variable.
    UiCvarUpdate = 51,
    /// Remaining memory.
    UiMemoryRemaining = 52,
    /// Get the CD key.
    UiGetCdkey = 53,
    /// Set the CD key.
    UiSetCdkey = 54,
    /// Register a font.
    UiRRegisterfont = 55,
    /// Model bounds.
    UiRModelbounds = 56,
    /// Add a global precompiler define.
    UiPcAddGlobalDefine = 57,
    /// Load a precompiler source.
    UiPcLoadSource = 58,
    /// Free a precompiler source.
    UiPcFreeSource = 59,
    /// Read a precompiler token.
    UiPcReadToken = 60,
    /// Precompiler source file and line.
    UiPcSourceFileAndLine = 61,
    /// Stop the background track.
    UiSStopbackgroundtrack = 62,
    /// Start the background track.
    UiSStartbackgroundtrack = 63,
    /// Real time.
    UiRealTime = 64,
    /// LAN server count.
    UiLanGetservercount = 65,
    /// LAN server address.
    UiLanGetserveraddressstring = 66,
    /// LAN server info.
    UiLanGetserverinfo = 67,
    /// Mark a server visible.
    UiLanMarkservervisible = 68,
    /// Update visible pings.
    UiLanUpdatevisiblepings = 69,
    /// Reset pings.
    UiLanResetpings = 70,
    /// Load cached servers.
    UiLanLoadcachedservers = 71,
    /// Save cached servers.
    UiLanSavecachedservers = 72,
    /// Add a server.
    UiLanAddserver = 73,
    /// Remove a server.
    UiLanRemoveserver = 74,
    /// Play a cinematic.
    UiCinPlaycinematic = 75,
    /// Stop a cinematic.
    UiCinStopcinematic = 76,
    /// Run a cinematic.
    UiCinRuncinematic = 77,
    /// Draw a cinematic.
    UiCinDrawcinematic = 78,
    /// Set cinematic extents.
    UiCinSetextents = 79,
    /// Remap a shader.
    UiRRemapShader = 80,
    /// Verify the CD key.
    UiVerifyCdkey = 81,
    /// LAN server status.
    UiLanServerstatus = 82,
    /// LAN server ping.
    UiLanGetserverping = 83,
    /// Server visibility test.
    UiLanServerisvisible = 84,
    /// Compare servers.
    UiLanCompareservers = 85,
    /// Seek a file.
    UiFsSeek = 86,
    /// Set PunkBuster client status.
    UiSetPbclstatus = 87,
    /// Memory set.
    UiMemset = 100,
    /// Memory copy.
    UiMemcpy = 101,
    /// Bounded string copy.
    UiStrncpy = 102,
    /// Sine.
    UiSin = 103,
    /// Cosine.
    UiCos = 104,
    /// Arc tangent.
    UiAtan2 = 105,
    /// Square root.
    UiSqrt = 106,
    /// Floor.
    UiFloor = 107,
    /// Ceiling.
    UiCeil = 108,
}

/// Decode a UI trap word.
#[must_use]
pub fn decode_qvm_ui_import(word: i32) -> Option<QvmUiImport> {
    use QvmUiImport as U;
    match word {
        0 => Some(U::UiError),
        1 => Some(U::UiPrint),
        2 => Some(U::UiMilliseconds),
        3 => Some(U::UiCvarSet),
        4 => Some(U::UiCvarVariablevalue),
        5 => Some(U::UiCvarVariablestringbuffer),
        6 => Some(U::UiCvarSetvalue),
        7 => Some(U::UiCvarReset),
        8 => Some(U::UiCvarCreate),
        9 => Some(U::UiCvarInfostringbuffer),
        10 => Some(U::UiArgc),
        11 => Some(U::UiArgv),
        12 => Some(U::UiCmdExecutetext),
        13 => Some(U::UiFsFopenfile),
        14 => Some(U::UiFsRead),
        15 => Some(U::UiFsWrite),
        16 => Some(U::UiFsFclosefile),
        17 => Some(U::UiFsGetfilelist),
        18 => Some(U::UiRRegistermodel),
        19 => Some(U::UiRRegisterskin),
        20 => Some(U::UiRRegistershadernomip),
        21 => Some(U::UiRClearscene),
        22 => Some(U::UiRAddrefentitytoscene),
        23 => Some(U::UiRAddpolytoscene),
        24 => Some(U::UiRAddlighttoscene),
        25 => Some(U::UiRenderscene),
        26 => Some(U::UiRSetcolor),
        27 => Some(U::UiRDrawstretchpic),
        28 => Some(U::UiUpdatescreen),
        29 => Some(U::UiCmLerptag),
        30 => Some(U::UiCmLoadmodel),
        31 => Some(U::UiSRegistersound),
        32 => Some(U::UiSStartlocalsound),
        33 => Some(U::UiKeyKeynumtostringbuf),
        34 => Some(U::UiKeyGetbindingbuf),
        35 => Some(U::UiKeySetbinding),
        36 => Some(U::UiKeyIsdown),
        37 => Some(U::UiKeyGetoverstrikemode),
        38 => Some(U::UiKeySetoverstrikemode),
        39 => Some(U::UiKeyClearstates),
        40 => Some(U::UiKeyGetcatcher),
        41 => Some(U::UiKeySetcatcher),
        42 => Some(U::UiGetclipboarddata),
        43 => Some(U::UiGetglconfig),
        44 => Some(U::UiGetclientstate),
        45 => Some(U::UiGetconfigstring),
        46 => Some(U::UiLanGetpingqueuecount),
        47 => Some(U::UiLanClearping),
        48 => Some(U::UiLanGetping),
        49 => Some(U::UiLanGetpinginfo),
        50 => Some(U::UiCvarRegister),
        51 => Some(U::UiCvarUpdate),
        52 => Some(U::UiMemoryRemaining),
        53 => Some(U::UiGetCdkey),
        54 => Some(U::UiSetCdkey),
        55 => Some(U::UiRRegisterfont),
        56 => Some(U::UiRModelbounds),
        57 => Some(U::UiPcAddGlobalDefine),
        58 => Some(U::UiPcLoadSource),
        59 => Some(U::UiPcFreeSource),
        60 => Some(U::UiPcReadToken),
        61 => Some(U::UiPcSourceFileAndLine),
        62 => Some(U::UiSStopbackgroundtrack),
        63 => Some(U::UiSStartbackgroundtrack),
        64 => Some(U::UiRealTime),
        65 => Some(U::UiLanGetservercount),
        66 => Some(U::UiLanGetserveraddressstring),
        67 => Some(U::UiLanGetserverinfo),
        68 => Some(U::UiLanMarkservervisible),
        69 => Some(U::UiLanUpdatevisiblepings),
        70 => Some(U::UiLanResetpings),
        71 => Some(U::UiLanLoadcachedservers),
        72 => Some(U::UiLanSavecachedservers),
        73 => Some(U::UiLanAddserver),
        74 => Some(U::UiLanRemoveserver),
        75 => Some(U::UiCinPlaycinematic),
        76 => Some(U::UiCinStopcinematic),
        77 => Some(U::UiCinRuncinematic),
        78 => Some(U::UiCinDrawcinematic),
        79 => Some(U::UiCinSetextents),
        80 => Some(U::UiRRemapShader),
        81 => Some(U::UiVerifyCdkey),
        82 => Some(U::UiLanServerstatus),
        83 => Some(U::UiLanGetserverping),
        84 => Some(U::UiLanServerisvisible),
        85 => Some(U::UiLanCompareservers),
        86 => Some(U::UiFsSeek),
        87 => Some(U::UiSetPbclstatus),
        100 => Some(U::UiMemset),
        101 => Some(U::UiMemcpy),
        102 => Some(U::UiStrncpy),
        103 => Some(U::UiSin),
        104 => Some(U::UiCos),
        105 => Some(U::UiAtan2),
        106 => Some(U::UiSqrt),
        107 => Some(U::UiFloor),
        108 => Some(U::UiCeil),
        _ => None,
    }
}

/// UI exports called by the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum QvmUiExport {
    /// Query the API version.
    UiGetapiversion = 0,
    /// Initialize the UI.
    UiInit = 1,
    /// Shut down the UI.
    UiShutdown = 2,
    /// A key event.
    UiKeyEvent = 3,
    /// A mouse event.
    UiMouseEvent = 4,
    /// Refresh the UI.
    UiRefresh = 5,
    /// Fullscreen test.
    UiIsFullscreen = 6,
    /// Set the active menu.
    UiSetActiveMenu = 7,
    /// A console command.
    UiConsoleCommand = 8,
    /// Draw the connect screen.
    UiDrawConnectScreen = 9,
    /// Unique CD key test.
    UiHasuniquecdkey = 10,
}

/// UI menus (`UIMENU_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum QvmUiMenu {
    /// No menu.
    UimenuNone = 0,
    /// Main menu.
    UimenuMain = 1,
    /// In-game menu.
    UimenuIngame = 2,
    /// Need-CD screen.
    UimenuNeedCd = 3,
    /// Bad-CD-key screen.
    UimenuBadCdKey = 4,
    /// Team menu.
    UimenuTeam = 5,
    /// Post-game menu.
    UimenuPostgame = 6,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn game_imports_decode_with_botlib_gaps() {
        assert_eq!(decode_qvm_game_import(0), Some(QvmGameImport::GPrint));
        assert_eq!(decode_qvm_game_import(45), Some(QvmGameImport::GFSeek));
        assert_eq!(decode_qvm_game_import(46), None);
        assert_eq!(decode_qvm_game_import(99), None);
        assert_eq!(decode_qvm_game_import(200), Some(QvmGameImport::BotlibSetup));
        assert_eq!(
            decode_qvm_game_import(318),
            Some(QvmGameImport::BotlibAasPredictClientMovement)
        );
        assert_eq!(decode_qvm_game_import(319), None);
        assert_eq!(
            decode_qvm_game_import(581),
            Some(QvmGameImport::BotlibPcSourceFileAndLine)
        );
        assert_eq!(decode_qvm_game_import(582), None);
        assert_eq!(QvmGameImport::GPrint as i32, 0);
        assert_eq!(QvmGameExport::BotaiStartFrame as i32, 10);
    }

    #[test]
    fn cgame_imports_decode_with_intrinsic_gap() {
        assert_eq!(decode_qvm_cgame_import(71), Some(QvmCgameImport::CgSnapvector));
        assert_eq!(decode_qvm_cgame_import(89), Some(QvmCgameImport::CgFsSeek));
        assert_eq!(decode_qvm_cgame_import(90), None);
        assert_eq!(decode_qvm_cgame_import(100), Some(QvmCgameImport::CgMemset));
        assert_eq!(decode_qvm_cgame_import(111), Some(QvmCgameImport::CgAcos));
        assert_eq!(decode_qvm_cgame_import(112), None);
        assert_eq!(QvmCgameExport::CgEventHandling as i32, 8);
    }

    #[test]
    fn ui_imports_decode_with_intrinsic_gap() {
        assert_eq!(decode_qvm_ui_import(0), Some(QvmUiImport::UiError));
        assert_eq!(decode_qvm_ui_import(87), Some(QvmUiImport::UiSetPbclstatus));
        assert_eq!(decode_qvm_ui_import(88), None);
        assert_eq!(decode_qvm_ui_import(108), Some(QvmUiImport::UiCeil));
        assert_eq!(decode_qvm_ui_import(-1), None);
        assert_eq!(QvmUiExport::UiHasuniquecdkey as i32, 10);
        assert_eq!(QvmUiMenu::UimenuPostgame as i32, 6);
    }
}
