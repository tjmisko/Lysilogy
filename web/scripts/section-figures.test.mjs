import assert from 'node:assert/strict';
import test from 'node:test';
import { externalSectionFigures } from '../src/lib/sectionFigures.ts';
const rect = y => ({x_min:50,x_max:300,y_min:y,y_max:y+10});
const page = number => ({number,width:600,height:800,tokens:[40,90,140,190].map((y,index)=>({index,line:index,text:'Text',rects:[rect(y)]})),sentences:[]});
const anchor = (number, token) => ({page:number,start_token:token,end_token:token,rects:[rect(40+50*token)],exact_text:'Text'});
const section = {pages:{start:1,end:1},source_span:{start:anchor(1,1),end:anchor(1,3)}};
const fig = (page, y, refPage=1, refY=90) => ({id:'f',page,label:'Figure 1',start:0,end:5,rect:rect(y),references:[{page:refPage,rects:[rect(refY)]}]});
const index = (figure) => ({figures:[figure],tokens:[{start:0,end:5,page:figure.page,rects:[figure.rect]}]});
test('figures cited inside the crop but rendered on another page are available',()=>{
  assert.equal(externalSectionFigures(index(fig(2,90)),section,[page(1),page(2)]).length,1);
});
test('same-page figure above the section is included; references above it are excluded',()=>{
  assert.equal(externalSectionFigures(index(fig(1,40)),section,[page(1)]).length,1);
  assert.equal(externalSectionFigures(index(fig(2,90,1,40)),section,[page(1),page(2)]).length,0);
});
test('figure and caption already inside the section do not duplicate',()=>{
  assert.equal(externalSectionFigures(index(fig(1,140)),section,[page(1)]).length,0);
});
