// Message-local presentation only. These snapshots never read a resource or
// grant authority to a tool, task, source, or retained/private field.
import type {TimelinePart} from './chatTimeline.ts';

export const MESSAGE_PART_READING = Symbol('message-part-reading');
export type ReadingField = 'input' | 'preview' | 'result';
export type DisclosureField = 'technical' | 'input' | 'group' | 'tool' | 'process';
export interface MessageTextSnapshot {readonly partKey:string; readonly field:ReadingField; readonly title:string; readonly text:string}
export interface PartReading {
    readonly identity:string;
    choice(field:DisclosureField):boolean|null;
    choose(field:DisclosureField, expanded:boolean):void;
    open(field:ReadingField,title:string,text:string,button:HTMLButtonElement):void;
}
export interface MessagePartReading {bind(part:TimelinePart):PartReading}
export function readingPartKey(part:TimelinePart):string {
    return JSON.stringify([part.source,part.source==='journal'?part.journalOwner:null,part.id,part.value.kind]);
}

/** Exact UI lifetimes fence delayed callbacks, including owner/Part ABA. */
export class MessageReadingScope {
    private owner:string;
    private lifetime={};
    private parts=new Map<string,{identity:string}>();
    private nextIdentity=0;
    private active=true;
    private choices=new Map<string,Partial<Record<DisclosureField,boolean>>>();
    selected:MessageTextSnapshot|null=null;
    constructor(owner:string,keys:Iterable<string>,active=true) {this.owner=owner;this.active=active;this.update(owner,keys,active);}
    update(owner:string,keys:Iterable<string>,active=true):boolean {
        let changed=false;
        if(owner!==this.owner){this.owner=owner;this.lifetime={};this.parts.clear();this.choices.clear();this.selected=null;changed=true;}
        if(active!==this.active){this.active=active;this.lifetime={};this.parts.clear();this.selected=null;changed=true;}
        const present=new Set(keys);
        for(const key of this.parts.keys())if(!present.has(key)){this.parts.delete(key);this.choices.delete(key);changed=true;}
        for(const key of this.choices.keys())if(!present.has(key)){this.choices.delete(key);changed=true;}
        if(active)for(const key of present)if(!this.parts.has(key)){this.parts.set(key,{identity:String(++this.nextIdentity)});changed=true;}
        if(this.selected&&!present.has(this.selected.partKey)){this.selected=null;changed=true;}
        return changed;
    }
    bind(partKey:string) {
        const lifetime=this.lifetime,part=this.parts.get(partKey);
        const current=()=>this.active&&!!part&&lifetime===this.lifetime&&this.parts.get(partKey)===part;
        return {
            current,
            identity:part?.identity??'',
            choice:(field:DisclosureField):boolean|null=>current()?this.choices.get(partKey)?.[field]??null:null,
            choose:(field:DisclosureField,expanded:boolean):boolean=>{
                if(!current())return false;
                this.choices.set(partKey,{...this.choices.get(partKey),[field]:expanded});return true;
            },
            open:(field:ReadingField,title:string,text:string):MessageTextSnapshot|null=>{
                if(!current())return null;
                return this.selected=Object.freeze({partKey,field,title,text});
            },
        };
    }
    close(snapshot:MessageTextSnapshot):boolean {
        if(this.selected!==snapshot)return false;
        this.selected=null;return true;
    }
    dispose():void {this.lifetime={};this.parts.clear();this.choices.clear();this.selected=null;}
}
