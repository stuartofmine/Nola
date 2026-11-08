#!/usr/bin/env python3
import copy
import unittest
from a2ui_real_multiagent import assert_surfaces

class SurfaceIsolationOracleTests(unittest.TestCase):
    def setUp(self):
        self.members = [{'id':'a','context_conversation_id':'ca'},{'id':'b','context_conversation_id':'cb'}]
        self.markers = ['MARKER_A','MARKER_B']
        self.surfaces = [{'phase':'ready','identity':{'owner':{'conversation_id':m['context_conversation_id'],
            'context_id':m['context_conversation_id'],'participant_id':m['id']},'surface_id':'shared-acceptance-form'},
            'nodes':{'name':{'value':marker}}} for m,marker in zip(self.members,self.markers)]
    def test_independent_owner_same_wire_name(self):
        assert_surfaces(self.members,self.surfaces,self.markers)
    def test_cross_member_value_is_rejected(self):
        self.surfaces[1]['nodes']['other']={'value':self.markers[0]}
        with self.assertRaises(AssertionError): assert_surfaces(self.members,self.surfaces,self.markers)
    def test_context_and_participant_cannot_be_inferred_from_surface_name(self):
        for field in ('context_id','participant_id','conversation_id'):
            surfaces=copy.deepcopy(self.surfaces)
            surfaces[1]['identity']['owner'][field]=surfaces[0]['identity']['owner'][field]
            with self.assertRaises(AssertionError): assert_surfaces(self.members,surfaces,self.markers)
    def test_model_ignoring_requested_wire_name_is_not_passed(self):
        self.surfaces[1]['identity']['surface_id']='different'
        with self.assertRaises(AssertionError): assert_surfaces(self.members,self.surfaces,self.markers)

if __name__ == '__main__': unittest.main()
