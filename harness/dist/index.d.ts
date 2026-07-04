export interface WsConfig {
    /** HTTP server URL (e.g. "http://localhost:8321") */
    serverUrl: string;
}
export declare function createWsTools(config: WsConfig): ((import("@earendil-works/pi-coding-agent").ToolDefinition<import("@sinclair/typebox").TObject<{
    path: import("@sinclair/typebox").TString;
}>, {
    size: any;
}, any> & import("@earendil-works/pi-coding-agent").ToolDefinition<any, any, any>) | (import("@earendil-works/pi-coding-agent").ToolDefinition<import("@sinclair/typebox").TObject<{
    command: import("@sinclair/typebox").TString;
}>, {
    exitCode: any;
    stdout: any;
    stderr: any;
}, any> & import("@earendil-works/pi-coding-agent").ToolDefinition<any, any, any>) | (import("@earendil-works/pi-coding-agent").ToolDefinition<import("@sinclair/typebox").TObject<{}>, {}, any> & import("@earendil-works/pi-coding-agent").ToolDefinition<any, any, any>))[];
