//! Legacy bot ABI import numbers and symbolic trap constants.
//!
//! Provenance: `src/compat/qvm/legacy-bot-abi.ts`. The `G_*`, `BOTLIB_*`,
//! `CG_*`, and `UI_*` constants mirror the numeric values of `QvmGameImport`,
//! `QvmCgameImport`, and `QvmUiImport` from `src/compat/qvm/abi.ts` (owned by
//! another worker); sibling files in this port reuse them via
//! `super::legacy_bot_abi::...`.

// MARK: qagame engine imports (QvmGameImport 0..=45).
/// Print trap.
pub const G_PRINT: i32 = 0;
/// Error trap.
pub const G_ERROR: i32 = 1;
/// Milliseconds trap.
pub const G_MILLISECONDS: i32 = 2;
/// Cvar register trap.
pub const G_CVAR_REGISTER: i32 = 3;
/// Cvar update trap.
pub const G_CVAR_UPDATE: i32 = 4;
/// Cvar set trap.
pub const G_CVAR_SET: i32 = 5;
/// Cvar integer-value trap.
pub const G_CVAR_VARIABLE_INTEGER_VALUE: i32 = 6;
/// Cvar string-buffer trap.
pub const G_CVAR_VARIABLE_STRING_BUFFER: i32 = 7;
/// Filesystem open trap.
pub const G_FS_FOPEN_FILE: i32 = 10;
/// Filesystem read trap.
pub const G_FS_READ: i32 = 11;
/// Filesystem write trap.
pub const G_FS_WRITE: i32 = 12;
/// Filesystem close trap.
pub const G_FS_FCLOSE_FILE: i32 = 13;
/// Drop-client trap.
pub const G_DROP_CLIENT: i32 = 16;
/// Send server command trap.
pub const G_SEND_SERVER_COMMAND: i32 = 17;
/// Set configstring trap.
pub const G_SET_CONFIGSTRING: i32 = 18;
/// Get configstring trap.
pub const G_GET_CONFIGSTRING: i32 = 19;
/// Get userinfo trap.
pub const G_GET_USERINFO: i32 = 20;
/// Set userinfo trap.
pub const G_SET_USERINFO: i32 = 21;
/// Get serverinfo trap.
pub const G_GET_SERVERINFO: i32 = 22;
/// Set brush model trap.
pub const G_SET_BRUSH_MODEL: i32 = 23;
/// Trace trap.
pub const G_TRACE: i32 = 24;
/// Point-contents trap.
pub const G_POINT_CONTENTS: i32 = 25;
/// In-PVS trap.
pub const G_IN_PVS: i32 = 26;
/// In-PVS ignoring portals trap.
pub const G_IN_PVS_IGNORE_PORTALS: i32 = 27;
/// Adjust area-portal state trap.
pub const G_ADJUST_AREA_PORTAL_STATE: i32 = 28;
/// Areas-connected trap.
pub const G_AREAS_CONNECTED: i32 = 29;
/// Link entity trap.
pub const G_LINKENTITY: i32 = 30;
/// Unlink entity trap.
pub const G_UNLINKENTITY: i32 = 31;
/// Entities-in-box trap.
pub const G_ENTITIES_IN_BOX: i32 = 32;
/// Entity-contact trap.
pub const G_ENTITY_CONTACT: i32 = 33;
/// Bot allocate-client trap.
pub const G_BOT_ALLOCATE_CLIENT: i32 = 34;
/// Bot free-client trap.
pub const G_BOT_FREE_CLIENT: i32 = 35;
/// Get user command trap.
pub const G_GET_USERCMD: i32 = 36;
/// Get entity token trap.
pub const G_GET_ENTITY_TOKEN: i32 = 37;
/// Filesystem list trap.
pub const G_FS_GETFILELIST: i32 = 38;
/// Capsule trace trap.
pub const G_TRACECAPSULE: i32 = 43;
/// Capsule entity-contact trap.
pub const G_ENTITY_CONTACTCAPSULE: i32 = 44;
/// Filesystem seek trap.
pub const G_FS_SEEK: i32 = 45;

// MARK: botlib setup/AAS imports (200..=211, 300..=318).
/// Botlib setup trap.
pub const BOTLIB_SETUP: i32 = 200;
/// Botlib shutdown trap.
pub const BOTLIB_SHUTDOWN: i32 = 201;
/// Botlib libvar-set trap.
pub const BOTLIB_LIBVAR_SET: i32 = 202;
/// Botlib libvar-get trap.
pub const BOTLIB_LIBVAR_GET: i32 = 203;
/// Botlib add-global-define trap.
pub const BOTLIB_PC_ADD_GLOBAL_DEFINE: i32 = 204;
/// Botlib start-frame trap.
pub const BOTLIB_START_FRAME: i32 = 205;
/// Botlib load-map trap.
pub const BOTLIB_LOAD_MAP: i32 = 206;
/// Botlib update-entity trap.
pub const BOTLIB_UPDATENTITY: i32 = 207;
/// Botlib test trap.
pub const BOTLIB_TEST: i32 = 208;
/// Botlib snapshot-entity trap.
pub const BOTLIB_GET_SNAPSHOT_ENTITY: i32 = 209;
/// Botlib console-message trap.
pub const BOTLIB_GET_CONSOLE_MESSAGE: i32 = 210;
/// Botlib user-command trap.
pub const BOTLIB_USER_COMMAND: i32 = 211;
/// AAS enable-routing-area trap.
pub const BOTLIB_AAS_ENABLE_ROUTING_AREA: i32 = 300;
/// AAS bbox-areas trap.
pub const BOTLIB_AAS_BBOX_AREAS: i32 = 301;
/// AAS area-info trap.
pub const BOTLIB_AAS_AREA_INFO: i32 = 302;
/// AAS entity-info trap.
pub const BOTLIB_AAS_ENTITY_INFO: i32 = 303;
/// AAS initialized trap.
pub const BOTLIB_AAS_INITIALIZED: i32 = 304;
/// AAS presence-bounds trap.
pub const BOTLIB_AAS_PRESENCE_TYPE_BOUNDING_BOX: i32 = 305;
/// AAS time trap.
pub const BOTLIB_AAS_TIME: i32 = 306;
/// AAS point-area trap.
pub const BOTLIB_AAS_POINT_AREA_NUM: i32 = 307;
/// AAS trace-areas trap.
pub const BOTLIB_AAS_TRACE_AREAS: i32 = 308;
/// AAS point-contents trap.
pub const BOTLIB_AAS_POINT_CONTENTS: i32 = 309;
/// AAS next-BSP-entity trap.
pub const BOTLIB_AAS_NEXT_BSP_ENTITY: i32 = 310;
/// AAS epair-value trap.
pub const BOTLIB_AAS_VALUE_FOR_BSP_EPAIR_KEY: i32 = 311;
/// AAS epair-vector trap.
pub const BOTLIB_AAS_VECTOR_FOR_BSP_EPAIR_KEY: i32 = 312;
/// AAS epair-float trap.
pub const BOTLIB_AAS_FLOAT_FOR_BSP_EPAIR_KEY: i32 = 313;
/// AAS epair-int trap.
pub const BOTLIB_AAS_INT_FOR_BSP_EPAIR_KEY: i32 = 314;
/// AAS area-reachability trap.
pub const BOTLIB_AAS_AREA_REACHABILITY: i32 = 315;
/// AAS travel-time trap.
pub const BOTLIB_AAS_AREA_TRAVEL_TIME_TO_GOAL_AREA: i32 = 316;
/// AAS swimming trap.
pub const BOTLIB_AAS_SWIMMING: i32 = 317;
/// AAS predict-movement trap.
pub const BOTLIB_AAS_PREDICT_CLIENT_MOVEMENT: i32 = 318;

// MARK: elementary-action imports (400..=423).
/// EA say trap.
pub const BOTLIB_EA_SAY: i32 = 400;
/// EA say-team trap.
pub const BOTLIB_EA_SAY_TEAM: i32 = 401;
/// EA command trap.
pub const BOTLIB_EA_COMMAND: i32 = 402;
/// EA action trap.
pub const BOTLIB_EA_ACTION: i32 = 403;
/// EA gesture trap.
pub const BOTLIB_EA_GESTURE: i32 = 404;
/// EA talk trap.
pub const BOTLIB_EA_TALK: i32 = 405;
/// EA attack trap.
pub const BOTLIB_EA_ATTACK: i32 = 406;
/// EA use trap.
pub const BOTLIB_EA_USE: i32 = 407;
/// EA respawn trap.
pub const BOTLIB_EA_RESPAWN: i32 = 408;
/// EA crouch trap.
pub const BOTLIB_EA_CROUCH: i32 = 409;
/// EA move-up trap.
pub const BOTLIB_EA_MOVE_UP: i32 = 410;
/// EA move-down trap.
pub const BOTLIB_EA_MOVE_DOWN: i32 = 411;
/// EA move-forward trap.
pub const BOTLIB_EA_MOVE_FORWARD: i32 = 412;
/// EA move-back trap.
pub const BOTLIB_EA_MOVE_BACK: i32 = 413;
/// EA move-left trap.
pub const BOTLIB_EA_MOVE_LEFT: i32 = 414;
/// EA move-right trap.
pub const BOTLIB_EA_MOVE_RIGHT: i32 = 415;
/// EA select-weapon trap.
pub const BOTLIB_EA_SELECT_WEAPON: i32 = 416;
/// EA jump trap.
pub const BOTLIB_EA_JUMP: i32 = 417;
/// EA delayed-jump trap.
pub const BOTLIB_EA_DELAYED_JUMP: i32 = 418;
/// EA move trap.
pub const BOTLIB_EA_MOVE: i32 = 419;
/// EA view trap.
pub const BOTLIB_EA_VIEW: i32 = 420;
/// EA end-regular trap.
pub const BOTLIB_EA_END_REGULAR: i32 = 421;
/// EA get-input trap.
pub const BOTLIB_EA_GET_INPUT: i32 = 422;
/// EA reset-input trap.
pub const BOTLIB_EA_RESET_INPUT: i32 = 423;

// MARK: AI imports (500..=581).
/// AI load-character trap.
pub const BOTLIB_AI_LOAD_CHARACTER: i32 = 500;
/// AI free-character trap.
pub const BOTLIB_AI_FREE_CHARACTER: i32 = 501;
/// AI characteristic-float trap.
pub const BOTLIB_AI_CHARACTERISTIC_FLOAT: i32 = 502;
/// AI characteristic-bounded-float trap.
pub const BOTLIB_AI_CHARACTERISTIC_BFLOAT: i32 = 503;
/// AI characteristic-integer trap.
pub const BOTLIB_AI_CHARACTERISTIC_INTEGER: i32 = 504;
/// AI characteristic-bounded-integer trap.
pub const BOTLIB_AI_CHARACTERISTIC_BINTEGER: i32 = 505;
/// AI characteristic-string trap.
pub const BOTLIB_AI_CHARACTERISTIC_STRING: i32 = 506;
/// AI alloc-chat-state trap.
pub const BOTLIB_AI_ALLOC_CHAT_STATE: i32 = 507;
/// AI free-chat-state trap.
pub const BOTLIB_AI_FREE_CHAT_STATE: i32 = 508;
/// AI queue-console-message trap.
pub const BOTLIB_AI_QUEUE_CONSOLE_MESSAGE: i32 = 509;
/// AI remove-console-message trap.
pub const BOTLIB_AI_REMOVE_CONSOLE_MESSAGE: i32 = 510;
/// AI next-console-message trap.
pub const BOTLIB_AI_NEXT_CONSOLE_MESSAGE: i32 = 511;
/// AI num-console-messages trap.
pub const BOTLIB_AI_NUM_CONSOLE_MESSAGE: i32 = 512;
/// AI initial-chat trap.
pub const BOTLIB_AI_INITIAL_CHAT: i32 = 513;
/// AI reply-chat trap.
pub const BOTLIB_AI_REPLY_CHAT: i32 = 514;
/// AI chat-length trap.
pub const BOTLIB_AI_CHAT_LENGTH: i32 = 515;
/// AI enter-chat trap.
pub const BOTLIB_AI_ENTER_CHAT: i32 = 516;
/// AI string-contains trap.
pub const BOTLIB_AI_STRING_CONTAINS: i32 = 517;
/// AI find-match trap.
pub const BOTLIB_AI_FIND_MATCH: i32 = 518;
/// AI match-variable trap.
pub const BOTLIB_AI_MATCH_VARIABLE: i32 = 519;
/// AI unify-white-spaces trap.
pub const BOTLIB_AI_UNIFY_WHITE_SPACES: i32 = 520;
/// AI replace-synonyms trap.
pub const BOTLIB_AI_REPLACE_SYNONYMS: i32 = 521;
/// AI load-chat-file trap.
pub const BOTLIB_AI_LOAD_CHAT_FILE: i32 = 522;
/// AI set-chat-gender trap.
pub const BOTLIB_AI_SET_CHAT_GENDER: i32 = 523;
/// AI set-chat-name trap.
pub const BOTLIB_AI_SET_CHAT_NAME: i32 = 524;
/// AI reset-goal-state trap.
pub const BOTLIB_AI_RESET_GOAL_STATE: i32 = 525;
/// AI reset-avoid-goals trap.
pub const BOTLIB_AI_RESET_AVOID_GOALS: i32 = 526;
/// AI push-goal trap.
pub const BOTLIB_AI_PUSH_GOAL: i32 = 527;
/// AI pop-goal trap.
pub const BOTLIB_AI_POP_GOAL: i32 = 528;
/// AI empty-goal-stack trap.
pub const BOTLIB_AI_EMPTY_GOAL_STACK: i32 = 529;
/// AI dump-avoid-goals trap.
pub const BOTLIB_AI_DUMP_AVOID_GOALS: i32 = 530;
/// AI dump-goal-stack trap.
pub const BOTLIB_AI_DUMP_GOAL_STACK: i32 = 531;
/// AI goal-name trap.
pub const BOTLIB_AI_GOAL_NAME: i32 = 532;
/// AI get-top-goal trap.
pub const BOTLIB_AI_GET_TOP_GOAL: i32 = 533;
/// AI get-second-goal trap.
pub const BOTLIB_AI_GET_SECOND_GOAL: i32 = 534;
/// AI choose-LTG-item trap.
pub const BOTLIB_AI_CHOOSE_LTG_ITEM: i32 = 535;
/// AI choose-NBG-item trap.
pub const BOTLIB_AI_CHOOSE_NBG_ITEM: i32 = 536;
/// AI touching-goal trap.
pub const BOTLIB_AI_TOUCHING_GOAL: i32 = 537;
/// AI item-goal-visibility trap.
pub const BOTLIB_AI_ITEM_GOAL_IN_VIS_BUT_NOT_VISIBLE: i32 = 538;
/// AI level-item-goal trap.
pub const BOTLIB_AI_GET_LEVEL_ITEM_GOAL: i32 = 539;
/// AI avoid-goal-time trap.
pub const BOTLIB_AI_AVOID_GOAL_TIME: i32 = 540;
/// AI init-level-items trap.
pub const BOTLIB_AI_INIT_LEVEL_ITEMS: i32 = 541;
/// AI update-entity-items trap.
pub const BOTLIB_AI_UPDATE_ENTITY_ITEMS: i32 = 542;
/// AI load-item-weights trap.
pub const BOTLIB_AI_LOAD_ITEM_WEIGHTS: i32 = 543;
/// AI free-item-weights trap.
pub const BOTLIB_AI_FREE_ITEM_WEIGHTS: i32 = 544;
/// AI save-goal-fuzzy-logic trap.
pub const BOTLIB_AI_SAVE_GOAL_FUZZY_LOGIC: i32 = 545;
/// AI alloc-goal-state trap.
pub const BOTLIB_AI_ALLOC_GOAL_STATE: i32 = 546;
/// AI free-goal-state trap.
pub const BOTLIB_AI_FREE_GOAL_STATE: i32 = 547;
/// AI reset-move-state trap.
pub const BOTLIB_AI_RESET_MOVE_STATE: i32 = 548;
/// AI move-to-goal trap.
pub const BOTLIB_AI_MOVE_TO_GOAL: i32 = 549;
/// AI move-in-direction trap.
pub const BOTLIB_AI_MOVE_IN_DIRECTION: i32 = 550;
/// AI reset-avoid-reach trap.
pub const BOTLIB_AI_RESET_AVOID_REACH: i32 = 551;
/// AI reset-last-avoid-reach trap.
pub const BOTLIB_AI_RESET_LAST_AVOID_REACH: i32 = 552;
/// AI reachability-area trap.
pub const BOTLIB_AI_REACHABILITY_AREA: i32 = 553;
/// AI movement-view-target trap.
pub const BOTLIB_AI_MOVEMENT_VIEW_TARGET: i32 = 554;
/// AI alloc-move-state trap.
pub const BOTLIB_AI_ALLOC_MOVE_STATE: i32 = 555;
/// AI free-move-state trap.
pub const BOTLIB_AI_FREE_MOVE_STATE: i32 = 556;
/// AI init-move-state trap.
pub const BOTLIB_AI_INIT_MOVE_STATE: i32 = 557;
/// AI choose-best-fight-weapon trap.
pub const BOTLIB_AI_CHOOSE_BEST_FIGHT_WEAPON: i32 = 558;
/// AI get-weapon-info trap.
pub const BOTLIB_AI_GET_WEAPON_INFO: i32 = 559;
/// AI load-weapon-weights trap.
pub const BOTLIB_AI_LOAD_WEAPON_WEIGHTS: i32 = 560;
/// AI alloc-weapon-state trap.
pub const BOTLIB_AI_ALLOC_WEAPON_STATE: i32 = 561;
/// AI free-weapon-state trap.
pub const BOTLIB_AI_FREE_WEAPON_STATE: i32 = 562;
/// AI reset-weapon-state trap.
pub const BOTLIB_AI_RESET_WEAPON_STATE: i32 = 563;
/// AI genetic-selection trap.
pub const BOTLIB_AI_GENETIC_PARENTS_AND_CHILD_SELECTION: i32 = 564;
/// AI interbreed-goal-fuzzy-logic trap.
pub const BOTLIB_AI_INTERBREED_GOAL_FUZZY_LOGIC: i32 = 565;
/// AI mutate-goal-fuzzy-logic trap.
pub const BOTLIB_AI_MUTATE_GOAL_FUZZY_LOGIC: i32 = 566;
/// AI next-camp-spot-goal trap.
pub const BOTLIB_AI_GET_NEXT_CAMP_SPOT_GOAL: i32 = 567;
/// AI map-location-goal trap.
pub const BOTLIB_AI_GET_MAP_LOCATION_GOAL: i32 = 568;
/// AI num-initial-chats trap.
pub const BOTLIB_AI_NUM_INITIAL_CHATS: i32 = 569;
/// AI get-chat-message trap.
pub const BOTLIB_AI_GET_CHAT_MESSAGE: i32 = 570;
/// AI remove-from-avoid-goals trap.
pub const BOTLIB_AI_REMOVE_FROM_AVOID_GOALS: i32 = 571;
/// AI predict-visible-position trap.
pub const BOTLIB_AI_PREDICT_VISIBLE_POSITION: i32 = 572;
/// AI set-avoid-goal-time trap.
pub const BOTLIB_AI_SET_AVOID_GOAL_TIME: i32 = 573;
/// AI add-avoid-spot trap.
pub const BOTLIB_AI_ADD_AVOID_SPOT: i32 = 574;
/// AAS alternative-route-goal trap.
pub const BOTLIB_AAS_ALTERNATIVE_ROUTE_GOAL: i32 = 575;
/// AAS predict-route trap.
pub const BOTLIB_AAS_PREDICT_ROUTE: i32 = 576;
/// AAS reachability-area-index trap.
pub const BOTLIB_AAS_POINT_REACHABILITY_AREA_INDEX: i32 = 577;
/// PC load-source trap.
pub const BOTLIB_PC_LOAD_SOURCE: i32 = 578;
/// PC free-source trap.
pub const BOTLIB_PC_FREE_SOURCE: i32 = 579;
/// PC read-token trap.
pub const BOTLIB_PC_READ_TOKEN: i32 = 580;
/// PC source-file-and-line trap.
pub const BOTLIB_PC_SOURCE_FILE_AND_LINE: i32 = 581;

// MARK: cgame engine imports (QvmCgameImport).
/// Cgame cvar-register trap.
pub const CG_CVAR_REGISTER: i32 = 3;
/// Cgame cvar-update trap.
pub const CG_CVAR_UPDATE: i32 = 4;
/// Cgame cvar-set trap.
pub const CG_CVAR_SET: i32 = 5;
/// Cgame cvar-string-buffer trap.
pub const CG_CVAR_VARIABLESTRINGBUFFER: i32 = 6;
/// Cgame filesystem-open trap.
pub const CG_FS_FOPENFILE: i32 = 10;
/// Cgame filesystem-read trap.
pub const CG_FS_READ: i32 = 11;
/// Cgame filesystem-write trap.
pub const CG_FS_WRITE: i32 = 12;
/// Cgame filesystem-close trap.
pub const CG_FS_FCLOSEFILE: i32 = 13;
/// Cgame load-map trap.
pub const CG_CM_LOADMAP: i32 = 18;
/// Cgame inline-model-count trap.
pub const CG_CM_NUMINLINEMODELS: i32 = 19;
/// Cgame inline-model trap.
pub const CG_CM_INLINEMODEL: i32 = 20;
/// Cgame temp-box-model trap.
pub const CG_CM_TEMPBOXMODEL: i32 = 22;
/// Cgame point-contents trap.
pub const CG_CM_POINTCONTENTS: i32 = 23;
/// Cgame transformed-point-contents trap.
pub const CG_CM_TRANSFORMEDPOINTCONTENTS: i32 = 24;
/// Cgame box-trace trap.
pub const CG_CM_BOXTRACE: i32 = 25;
/// Cgame transformed-box-trace trap.
pub const CG_CM_TRANSFORMEDBOXTRACE: i32 = 26;
/// Cgame mark-fragments trap.
pub const CG_CM_MARKFRAGMENTS: i32 = 27;
/// Cgame start-sound trap.
pub const CG_S_STARTSOUND: i32 = 28;
/// Cgame start-local-sound trap.
pub const CG_S_STARTLOCALSOUND: i32 = 29;
/// Cgame clear-looping-sounds trap.
pub const CG_S_CLEARLOOPINGSOUNDS: i32 = 30;
/// Cgame add-looping-sound trap.
pub const CG_S_ADDLOOPINGSOUND: i32 = 31;
/// Cgame update-entity-position trap.
pub const CG_S_UPDATEENTITYPOSITION: i32 = 32;
/// Cgame respatialize trap.
pub const CG_S_RESPATIALIZE: i32 = 33;
/// Cgame register-sound trap.
pub const CG_S_REGISTERSOUND: i32 = 34;
/// Cgame start-background-track trap.
pub const CG_S_STARTBACKGROUNDTRACK: i32 = 35;
/// Cgame load-world trap.
pub const CG_R_LOADWORLDMAP: i32 = 36;
/// Cgame register-model trap.
pub const CG_R_REGISTERMODEL: i32 = 37;
/// Cgame register-skin trap.
pub const CG_R_REGISTERSKIN: i32 = 38;
/// Cgame register-shader trap.
pub const CG_R_REGISTERSHADER: i32 = 39;
/// Cgame clear-scene trap.
pub const CG_R_CLEARSCENE: i32 = 40;
/// Cgame add-ref-entity trap.
pub const CG_R_ADDREFENTITYTOSCENE: i32 = 41;
/// Cgame add-poly trap.
pub const CG_R_ADDPOLYTOSCENE: i32 = 42;
/// Cgame add-light trap.
pub const CG_R_ADDLIGHTTOSCENE: i32 = 43;
/// Cgame render-scene trap.
pub const CG_R_RENDERSCENE: i32 = 44;
/// Cgame set-color trap.
pub const CG_R_SETCOLOR: i32 = 45;
/// Cgame draw-stretch-pic trap.
pub const CG_R_DRAWSTRETCHPIC: i32 = 46;
/// Cgame model-bounds trap.
pub const CG_R_MODELBOUNDS: i32 = 47;
/// Cgame lerp-tag trap.
pub const CG_R_LERPTAG: i32 = 48;
/// Cgame get-game-state trap.
pub const CG_GETGAMESTATE: i32 = 50;
/// Cgame get-current-snapshot-number trap.
pub const CG_GETCURRENTSNAPSHOTNUMBER: i32 = 51;
/// Cgame get-snapshot trap.
pub const CG_GETSNAPSHOT: i32 = 52;
/// Cgame get-server-command trap.
pub const CG_GETSERVERCOMMAND: i32 = 53;
/// Cgame get-current-command-number trap.
pub const CG_GETCURRENTCMDNUMBER: i32 = 54;
/// Cgame get-user-command trap.
pub const CG_GETUSERCMD: i32 = 55;
/// Cgame set-user-command-value trap.
pub const CG_SETUSERCMDVALUE: i32 = 56;
/// Cgame register-shader-nomip trap.
pub const CG_R_REGISTERSHADERNOMIP: i32 = 57;
/// Cgame add-global-define trap.
pub const CG_PC_ADD_GLOBAL_DEFINE: i32 = 64;
/// Cgame load-source trap.
pub const CG_PC_LOAD_SOURCE: i32 = 65;
/// Cgame free-source trap.
pub const CG_PC_FREE_SOURCE: i32 = 66;
/// Cgame read-token trap.
pub const CG_PC_READ_TOKEN: i32 = 67;
/// Cgame source-file-and-line trap.
pub const CG_PC_SOURCE_FILE_AND_LINE: i32 = 68;
/// Cgame stop-background-track trap.
pub const CG_S_STOPBACKGROUNDTRACK: i32 = 69;
/// Cgame play-cinematic trap.
pub const CG_CIN_PLAYCINEMATIC: i32 = 74;
/// Cgame stop-cinematic trap.
pub const CG_CIN_STOPCINEMATIC: i32 = 75;
/// Cgame run-cinematic trap.
pub const CG_CIN_RUNCINEMATIC: i32 = 76;
/// Cgame draw-cinematic trap.
pub const CG_CIN_DRAWCINEMATIC: i32 = 77;
/// Cgame set-cinematic-extents trap.
pub const CG_CIN_SETEXTENTS: i32 = 78;
/// Cgame remap-shader trap.
pub const CG_R_REMAP_SHADER: i32 = 79;
/// Cgame add-real-looping-sound trap.
pub const CG_S_ADDREALLOOPINGSOUND: i32 = 80;
/// Cgame stop-looping-sound trap.
pub const CG_S_STOPLOOPINGSOUND: i32 = 81;
/// Cgame temp-capsule-model trap.
pub const CG_CM_TEMPCAPSULEMODEL: i32 = 82;
/// Cgame capsule-trace trap.
pub const CG_CM_CAPSULETRACE: i32 = 83;
/// Cgame transformed-capsule-trace trap.
pub const CG_CM_TRANSFORMEDCAPSULETRACE: i32 = 84;
/// Cgame add-additive-light trap.
pub const CG_R_ADDADDITIVELIGHTTOSCENE: i32 = 85;
/// Cgame get-entity-token trap.
pub const CG_GET_ENTITY_TOKEN: i32 = 86;
/// Cgame add-polys trap.
pub const CG_R_ADDPOLYSTOSCENE: i32 = 87;
/// Cgame in-PVS trap.
pub const CG_R_INPVS: i32 = 88;
/// Cgame filesystem-seek trap.
pub const CG_FS_SEEK: i32 = 89;

// MARK: ui engine imports (QvmUiImport).
/// UI cvar-set trap.
pub const UI_CVAR_SET: i32 = 3;
/// UI cvar-value trap.
pub const UI_CVAR_VARIABLEVALUE: i32 = 4;
/// UI cvar-string-buffer trap.
pub const UI_CVAR_VARIABLESTRINGBUFFER: i32 = 5;
/// UI cvar-set-value trap.
pub const UI_CVAR_SETVALUE: i32 = 6;
/// UI cvar-reset trap.
pub const UI_CVAR_RESET: i32 = 7;
/// UI cvar-create trap.
pub const UI_CVAR_CREATE: i32 = 8;
/// UI cvar-infostring trap.
pub const UI_CVAR_INFOSTRINGBUFFER: i32 = 9;
/// UI filesystem-open trap.
pub const UI_FS_FOPENFILE: i32 = 13;
/// UI filesystem-read trap.
pub const UI_FS_READ: i32 = 14;
/// UI filesystem-write trap.
pub const UI_FS_WRITE: i32 = 15;
/// UI filesystem-close trap.
pub const UI_FS_FCLOSEFILE: i32 = 16;
/// UI filesystem-list trap.
pub const UI_FS_GETFILELIST: i32 = 17;
/// UI register-model trap.
pub const UI_R_REGISTERMODEL: i32 = 18;
/// UI register-skin trap.
pub const UI_R_REGISTERSKIN: i32 = 19;
/// UI register-shader-nomip trap.
pub const UI_R_REGISTERSHADERNOMIP: i32 = 20;
/// UI clear-scene trap.
pub const UI_R_CLEARSCENE: i32 = 21;
/// UI add-ref-entity trap.
pub const UI_R_ADDREFENTITYTOSCENE: i32 = 22;
/// UI add-poly trap.
pub const UI_R_ADDPOLYTOSCENE: i32 = 23;
/// UI add-light trap.
pub const UI_R_ADDLIGHTTOSCENE: i32 = 24;
/// UI render-scene trap.
pub const UI_R_RENDERSCENE: i32 = 25;
/// UI set-color trap.
pub const UI_R_SETCOLOR: i32 = 26;
/// UI draw-stretch-pic trap.
pub const UI_R_DRAWSTRETCHPIC: i32 = 27;
/// UI lerp-tag trap.
pub const UI_CM_LERPTAG: i32 = 29;
/// UI register-sound trap.
pub const UI_S_REGISTERSOUND: i32 = 31;
/// UI start-local-sound trap.
pub const UI_S_STARTLOCALSOUND: i32 = 32;
/// UI get-configstring trap.
pub const UI_GETCONFIGSTRING: i32 = 45;
/// UI ping-queue-count trap.
pub const UI_LAN_GETPINGQUEUECOUNT: i32 = 46;
/// UI clear-ping trap.
pub const UI_LAN_CLEARPING: i32 = 47;
/// UI get-ping trap.
pub const UI_LAN_GETPING: i32 = 48;
/// UI get-ping-info trap.
pub const UI_LAN_GETPINGINFO: i32 = 49;
/// UI cvar-register trap.
pub const UI_CVAR_REGISTER: i32 = 50;
/// UI cvar-update trap.
pub const UI_CVAR_UPDATE: i32 = 51;
/// UI get-CD-key trap.
pub const UI_GET_CDKEY: i32 = 53;
/// UI set-CD-key trap.
pub const UI_SET_CDKEY: i32 = 54;
/// UI model-bounds trap.
pub const UI_R_MODELBOUNDS: i32 = 56;
/// UI add-global-define trap.
pub const UI_PC_ADD_GLOBAL_DEFINE: i32 = 57;
/// UI load-source trap.
pub const UI_PC_LOAD_SOURCE: i32 = 58;
/// UI free-source trap.
pub const UI_PC_FREE_SOURCE: i32 = 59;
/// UI read-token trap.
pub const UI_PC_READ_TOKEN: i32 = 60;
/// UI source-file-and-line trap.
pub const UI_PC_SOURCE_FILE_AND_LINE: i32 = 61;
/// UI stop-background-track trap.
pub const UI_S_STOPBACKGROUNDTRACK: i32 = 62;
/// UI start-background-track trap.
pub const UI_S_STARTBACKGROUNDTRACK: i32 = 63;
/// UI get-server-count trap.
pub const UI_LAN_GETSERVERCOUNT: i32 = 65;
/// UI get-server-address trap.
pub const UI_LAN_GETSERVERADDRESSSTRING: i32 = 66;
/// UI get-server-info trap.
pub const UI_LAN_GETSERVERINFO: i32 = 67;
/// UI mark-server-visible trap.
pub const UI_LAN_MARKSERVERVISIBLE: i32 = 68;
/// UI update-visible-pings trap.
pub const UI_LAN_UPDATEVISIBLEPINGS: i32 = 69;
/// UI reset-pings trap.
pub const UI_LAN_RESETPINGS: i32 = 70;
/// UI load-cached-servers trap.
pub const UI_LAN_LOADCACHEDSERVERS: i32 = 71;
/// UI save-cached-servers trap.
pub const UI_LAN_SAVECACHEDSERVERS: i32 = 72;
/// UI add-server trap.
pub const UI_LAN_ADDSERVER: i32 = 73;
/// UI remove-server trap.
pub const UI_LAN_REMOVESERVER: i32 = 74;
/// UI play-cinematic trap.
pub const UI_CIN_PLAYCINEMATIC: i32 = 75;
/// UI stop-cinematic trap.
pub const UI_CIN_STOPCINEMATIC: i32 = 76;
/// UI run-cinematic trap.
pub const UI_CIN_RUNCINEMATIC: i32 = 77;
/// UI draw-cinematic trap.
pub const UI_CIN_DRAWCINEMATIC: i32 = 78;
/// UI set-cinematic-extents trap.
pub const UI_CIN_SETEXTENTS: i32 = 79;
/// UI remap-shader trap.
pub const UI_R_REMAP_SHADER: i32 = 80;
/// UI verify-CD-key trap.
pub const UI_VERIFY_CDKEY: i32 = 81;
/// UI server-status trap.
pub const UI_LAN_SERVERSTATUS: i32 = 82;
/// UI get-server-ping trap.
pub const UI_LAN_GETSERVERPING: i32 = 83;
/// UI server-is-visible trap.
pub const UI_LAN_SERVERISVISIBLE: i32 = 84;
/// UI compare-servers trap.
pub const UI_LAN_COMPARESERVERS: i32 = 85;
/// UI filesystem-seek trap.
pub const UI_FS_SEEK: i32 = 86;

// MARK: ui exports (QvmUiExport) and menus (QvmUiMenu).
/// UI init export.
pub const UI_INIT: i32 = 1;
/// UI shutdown export.
pub const UI_SHUTDOWN: i32 = 2;
/// UI key-event export.
pub const UI_KEY_EVENT: i32 = 3;
/// UI mouse-event export.
pub const UI_MOUSE_EVENT: i32 = 4;
/// UI refresh export.
pub const UI_REFRESH: i32 = 5;
/// UI is-fullscreen export.
pub const UI_IS_FULLSCREEN: i32 = 6;
/// UI set-active-menu export.
pub const UI_SET_ACTIVE_MENU: i32 = 7;
/// UI console-command export.
pub const UI_CONSOLE_COMMAND: i32 = 8;
/// UI draw-connect-screen export.
pub const UI_DRAW_CONNECT_SCREEN: i32 = 9;
/// UI has-unique-CD-key export.
pub const UI_HASUNIQUECDKEY: i32 = 10;
/// Menu: none.
pub const UIMENU_NONE: i32 = 0;
/// Menu: main.
pub const UIMENU_MAIN: i32 = 1;
/// Menu: ingame.
pub const UIMENU_INGAME: i32 = 2;
/// Menu: need-CD.
pub const UIMENU_NEED_CD: i32 = 3;
/// Menu: bad-CD-key.
pub const UIMENU_BAD_CD_KEY: i32 = 4;
/// Menu: team.
pub const UIMENU_TEAM: i32 = 5;
/// Menu: postgame.
pub const UIMENU_POSTGAME: i32 = 6;

/// Legacy trap number to modern `QvmGameImport` number.
///
/// Trap numbers 0 through 40 decode identically; higher numbers follow the
/// legacy bot-import table. Returns `None` for unmapped numbers.
pub fn decode_legacy_game_import(code: i32) -> Option<i32> {
    if (0..=40).contains(&code) {
        return Some(code);
    }
    LEGACY_BOT_IMPORTS
        .iter()
        .find_map(|(legacy, modern)| (*legacy == code).then_some(*modern))
}

/// Legacy trap number paired with its modern import number.
const LEGACY_BOT_IMPORTS: [(i32, i32); 124] = [
    (200, BOTLIB_SETUP),
    (201, BOTLIB_SHUTDOWN),
    (202, BOTLIB_LIBVAR_SET),
    (203, BOTLIB_LIBVAR_GET),
    (204, BOTLIB_PC_ADD_GLOBAL_DEFINE),
    (205, BOTLIB_START_FRAME),
    (206, BOTLIB_LOAD_MAP),
    (207, BOTLIB_UPDATENTITY),
    (208, BOTLIB_TEST),
    (209, BOTLIB_GET_SNAPSHOT_ENTITY),
    (210, BOTLIB_GET_CONSOLE_MESSAGE),
    (211, BOTLIB_USER_COMMAND),
    (303, BOTLIB_AAS_ENTITY_INFO),
    (304, BOTLIB_AAS_INITIALIZED),
    (305, BOTLIB_AAS_PRESENCE_TYPE_BOUNDING_BOX),
    (306, BOTLIB_AAS_TIME),
    (307, BOTLIB_AAS_POINT_AREA_NUM),
    (308, BOTLIB_AAS_TRACE_AREAS),
    (309, BOTLIB_AAS_POINT_CONTENTS),
    (310, BOTLIB_AAS_NEXT_BSP_ENTITY),
    (311, BOTLIB_AAS_VALUE_FOR_BSP_EPAIR_KEY),
    (312, BOTLIB_AAS_VECTOR_FOR_BSP_EPAIR_KEY),
    (313, BOTLIB_AAS_FLOAT_FOR_BSP_EPAIR_KEY),
    (314, BOTLIB_AAS_INT_FOR_BSP_EPAIR_KEY),
    (315, BOTLIB_AAS_AREA_REACHABILITY),
    (316, BOTLIB_AAS_AREA_TRAVEL_TIME_TO_GOAL_AREA),
    (317, BOTLIB_AAS_SWIMMING),
    (318, BOTLIB_AAS_PREDICT_CLIENT_MOVEMENT),
    (400, BOTLIB_EA_SAY),
    (401, BOTLIB_EA_SAY_TEAM),
    (406, BOTLIB_EA_GESTURE),
    (407, BOTLIB_EA_COMMAND),
    (408, BOTLIB_EA_SELECT_WEAPON),
    (409, BOTLIB_EA_TALK),
    (410, BOTLIB_EA_ATTACK),
    (411, BOTLIB_EA_USE),
    (412, BOTLIB_EA_RESPAWN),
    (413, BOTLIB_EA_JUMP),
    (414, BOTLIB_EA_DELAYED_JUMP),
    (415, BOTLIB_EA_CROUCH),
    (416, BOTLIB_EA_MOVE_UP),
    (417, BOTLIB_EA_MOVE_DOWN),
    (418, BOTLIB_EA_MOVE_FORWARD),
    (419, BOTLIB_EA_MOVE_BACK),
    (420, BOTLIB_EA_MOVE_LEFT),
    (421, BOTLIB_EA_MOVE_RIGHT),
    (422, BOTLIB_EA_MOVE),
    (423, BOTLIB_EA_VIEW),
    (424, BOTLIB_EA_END_REGULAR),
    (425, BOTLIB_EA_GET_INPUT),
    (426, BOTLIB_EA_RESET_INPUT),
    (500, BOTLIB_AI_LOAD_CHARACTER),
    (501, BOTLIB_AI_FREE_CHARACTER),
    (502, BOTLIB_AI_CHARACTERISTIC_FLOAT),
    (503, BOTLIB_AI_CHARACTERISTIC_BFLOAT),
    (504, BOTLIB_AI_CHARACTERISTIC_INTEGER),
    (505, BOTLIB_AI_CHARACTERISTIC_BINTEGER),
    (506, BOTLIB_AI_CHARACTERISTIC_STRING),
    (507, BOTLIB_AI_ALLOC_CHAT_STATE),
    (508, BOTLIB_AI_FREE_CHAT_STATE),
    (509, BOTLIB_AI_QUEUE_CONSOLE_MESSAGE),
    (510, BOTLIB_AI_REMOVE_CONSOLE_MESSAGE),
    (511, BOTLIB_AI_NEXT_CONSOLE_MESSAGE),
    (512, BOTLIB_AI_NUM_CONSOLE_MESSAGE),
    (513, BOTLIB_AI_INITIAL_CHAT),
    (514, BOTLIB_AI_REPLY_CHAT),
    (515, BOTLIB_AI_CHAT_LENGTH),
    (516, BOTLIB_AI_ENTER_CHAT),
    (517, BOTLIB_AI_STRING_CONTAINS),
    (518, BOTLIB_AI_FIND_MATCH),
    (519, BOTLIB_AI_MATCH_VARIABLE),
    (520, BOTLIB_AI_UNIFY_WHITE_SPACES),
    (521, BOTLIB_AI_REPLACE_SYNONYMS),
    (522, BOTLIB_AI_LOAD_CHAT_FILE),
    (523, BOTLIB_AI_SET_CHAT_GENDER),
    (524, BOTLIB_AI_SET_CHAT_NAME),
    (525, BOTLIB_AI_RESET_GOAL_STATE),
    (526, BOTLIB_AI_RESET_AVOID_GOALS),
    (527, BOTLIB_AI_PUSH_GOAL),
    (528, BOTLIB_AI_POP_GOAL),
    (529, BOTLIB_AI_EMPTY_GOAL_STACK),
    (530, BOTLIB_AI_DUMP_AVOID_GOALS),
    (531, BOTLIB_AI_DUMP_GOAL_STACK),
    (532, BOTLIB_AI_GOAL_NAME),
    (533, BOTLIB_AI_GET_TOP_GOAL),
    (534, BOTLIB_AI_GET_SECOND_GOAL),
    (535, BOTLIB_AI_CHOOSE_LTG_ITEM),
    (536, BOTLIB_AI_CHOOSE_NBG_ITEM),
    (537, BOTLIB_AI_TOUCHING_GOAL),
    (538, BOTLIB_AI_ITEM_GOAL_IN_VIS_BUT_NOT_VISIBLE),
    (539, BOTLIB_AI_GET_LEVEL_ITEM_GOAL),
    (540, BOTLIB_AI_AVOID_GOAL_TIME),
    (541, BOTLIB_AI_INIT_LEVEL_ITEMS),
    (542, BOTLIB_AI_UPDATE_ENTITY_ITEMS),
    (543, BOTLIB_AI_LOAD_ITEM_WEIGHTS),
    (544, BOTLIB_AI_FREE_ITEM_WEIGHTS),
    (545, BOTLIB_AI_SAVE_GOAL_FUZZY_LOGIC),
    (546, BOTLIB_AI_ALLOC_GOAL_STATE),
    (547, BOTLIB_AI_FREE_GOAL_STATE),
    (548, BOTLIB_AI_RESET_MOVE_STATE),
    (549, BOTLIB_AI_MOVE_TO_GOAL),
    (550, BOTLIB_AI_MOVE_IN_DIRECTION),
    (551, BOTLIB_AI_RESET_AVOID_REACH),
    (552, BOTLIB_AI_RESET_LAST_AVOID_REACH),
    (553, BOTLIB_AI_REACHABILITY_AREA),
    (554, BOTLIB_AI_MOVEMENT_VIEW_TARGET),
    (555, BOTLIB_AI_ALLOC_MOVE_STATE),
    (556, BOTLIB_AI_FREE_MOVE_STATE),
    (557, BOTLIB_AI_INIT_MOVE_STATE),
    (558, BOTLIB_AI_CHOOSE_BEST_FIGHT_WEAPON),
    (559, BOTLIB_AI_GET_WEAPON_INFO),
    (560, BOTLIB_AI_LOAD_WEAPON_WEIGHTS),
    (561, BOTLIB_AI_ALLOC_WEAPON_STATE),
    (562, BOTLIB_AI_FREE_WEAPON_STATE),
    (563, BOTLIB_AI_RESET_WEAPON_STATE),
    (564, BOTLIB_AI_GENETIC_PARENTS_AND_CHILD_SELECTION),
    (565, BOTLIB_AI_INTERBREED_GOAL_FUZZY_LOGIC),
    (566, BOTLIB_AI_MUTATE_GOAL_FUZZY_LOGIC),
    (567, BOTLIB_AI_GET_NEXT_CAMP_SPOT_GOAL),
    (568, BOTLIB_AI_GET_MAP_LOCATION_GOAL),
    (569, BOTLIB_AI_NUM_INITIAL_CHATS),
    (570, BOTLIB_AI_GET_CHAT_MESSAGE),
    (571, BOTLIB_AI_REMOVE_FROM_AVOID_GOALS),
    (572, BOTLIB_AI_PREDICT_VISIBLE_POSITION),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn low_traps_decode_identically() {
        assert_eq!(decode_legacy_game_import(0), Some(0));
        assert_eq!(decode_legacy_game_import(40), Some(40));
        assert_eq!(decode_legacy_game_import(G_GET_USERCMD), Some(36));
    }

    #[test]
    fn bot_ranges_follow_legacy_table() {
        assert_eq!(decode_legacy_game_import(200), Some(BOTLIB_SETUP));
        assert_eq!(decode_legacy_game_import(211), Some(BOTLIB_USER_COMMAND));
        assert_eq!(decode_legacy_game_import(303), Some(BOTLIB_AAS_ENTITY_INFO));
        assert_eq!(decode_legacy_game_import(318), Some(BOTLIB_AAS_PREDICT_CLIENT_MOVEMENT));
        assert_eq!(decode_legacy_game_import(572), Some(BOTLIB_AI_PREDICT_VISIBLE_POSITION));
    }

    #[test]
    fn legacy_ea_numbers_differ_from_modern() {
        assert_eq!(decode_legacy_game_import(406), Some(BOTLIB_EA_GESTURE));
        assert_eq!(decode_legacy_game_import(407), Some(BOTLIB_EA_COMMAND));
        assert_eq!(decode_legacy_game_import(402), None);
    }

    #[test]
    fn unmapped_numbers_are_none() {
        assert_eq!(decode_legacy_game_import(41), None);
        assert_eq!(decode_legacy_game_import(100), None);
        assert_eq!(decode_legacy_game_import(573), None);
        assert_eq!(decode_legacy_game_import(-1), None);
    }

    #[test]
    fn table_covers_expected_count() {
        assert_eq!(LEGACY_BOT_IMPORTS.len(), 124);
    }
}
