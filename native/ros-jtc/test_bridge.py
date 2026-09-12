#!/usr/bin/env python3
"""Real ROS action/service exchange with an explicitly simulated server, no robot drivers."""
import argparse,json,os,selectors,subprocess,tempfile,threading,time,uuid,hashlib
from pathlib import Path
import rclpy
from rclpy.action import ActionServer,GoalResponse,CancelResponse
from rclpy.callback_groups import ReentrantCallbackGroup
from rclpy.executors import MultiThreadedExecutor
from control_msgs.action import FollowJointTrajectory
from controller_manager_msgs.srv import ListControllers
from controller_manager_msgs.msg import ControllerState

parser=argparse.ArgumentParser();parser.add_argument('--binary',required=True);parser.add_argument('--evidence',required=True,type=Path);a=parser.parse_args()
DOMAIN=171
CATALOG=hashlib.sha256((Path(__file__).resolve().parents[2]/'catalogs/robotis-support.v1.json').read_bytes()).hexdigest()
os.environ['RMW_IMPLEMENTATION']='rmw_fastrtps_cpp';os.environ['ROS_AUTOMATIC_DISCOVERY_RANGE']='LOCALHOST';os.environ['ROS_LOG_DIR']='/tmp/ros-jtc-log'
JOINTS=['joint'+str(i) for i in range(1,7)]
rclpy.init(domain_id=DOMAIN)
node=rclpy.create_node('rx_test_only_jtc_server');group=ReentrantCallbackGroup()
state={'received':0,'accepted':0,'cancelled':0,'list_calls':0,'goals':[],'active':True,'extra_claim':False,'list_delay':0.0,'cancel_delay':0.0}
def lists(_request,response):
    state['list_calls']+=1;time.sleep(state['list_delay'])
    c=ControllerState();c.name='arm_controller';c.type='joint_trajectory_controller/JointTrajectoryController';c.state='active' if state['active'] else 'inactive'
    c.claimed_interfaces=[j+'/position' for j in JOINTS]+(['rh_r1_joint/position'] if state['extra_claim'] else [])
    response.controller=[c];return response
list_server=node.create_service(ListControllers,'/rx_test/controller_manager/list_controllers',lists,callback_group=group)
def accept(request):
    state['received']+=1
    if abs(request.trajectory.points[0].positions[0]-.6)<1e-6:time.sleep(.3)
    if abs(request.trajectory.points[0].positions[0]-.4)<1e-6:return GoalResponse.REJECT
    state['accepted']+=1;return GoalResponse.ACCEPT
def cancel(handle):
    state['cancelled']+=1;time.sleep(state['cancel_delay']);return CancelResponse.ACCEPT
def execute(handle):
    token=str(uuid.UUID(bytes=bytes(handle.goal_id.uuid)));state['goals'].append(token)
    value=handle.request.trajectory.points[0].positions[0];result=FollowJointTrajectory.Result()
    if abs(value-.3)<1e-6:
        end=time.monotonic()+4
        while not handle.is_cancel_requested and time.monotonic()<end:time.sleep(.01)
        if handle.is_cancel_requested:handle.canceled();return result
        handle.abort();result.error_code=-4;return result
    time.sleep(.3)
    if abs(value-.2)<1e-6:handle.abort();result.error_code=-4;result.error_string='test-only path tolerance violation'
    else:handle.succeed();result.error_code=-4 if abs(value-.5)<1e-6 else 0
    return result
server=ActionServer(node,FollowJointTrajectory,'/rx_test/arm_controller/follow_joint_trajectory',execute_callback=execute,goal_callback=accept,cancel_callback=cancel,callback_group=group)
executor=MultiThreadedExecutor(num_threads=4);executor.add_node(node);thread=threading.Thread(target=executor.spin,daemon=True);thread.start()

def goal(value=.1):
    tol=[{'name':j,'position':.1,'velocity':.1,'acceleration':.1} for j in JOINTS]
    return {'joints':JOINTS,'points':[{'positions':[value]*6,'velocities':[],'accelerations':[],'time_ns':'100000000'}],
            'path_tolerance':tol,'goal_tolerance':tol,'goal_time_ns':'100000000'}

class Bridge:
    def __init__(self,root,**changes):
        config={'schema':'rx.ros-jtc-bridge.v1','catalog_sha256':CATALOG,'support_id':'OM-06','controller':'arm_controller','namespace':'/rx_test','controller_manager':'/rx_test/controller_manager','domain_id':DOMAIN,'timeout_ms':150,'capacity':32};config.update(changes)
        file=root/(uuid.uuid4().hex+'.json');file.write_text(json.dumps(config));self.log=open(root/(file.stem+'.stderr'),'w+')
        self.p=subprocess.Popen([a.binary,str(file)],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=self.log,text=True,env=os.environ.copy());self.sequence=0
        self.hello=self.read();assert self.hello['state']=='READY';self.instance=self.hello['bridge_instance']
    def read(self):
        selector=selectors.DefaultSelector();selector.register(self.p.stdout,selectors.EVENT_READ)
        try:
            assert selector.select(8),'bridge response timeout'
            line=self.p.stdout.readline()
            if not line:self.log.seek(0);raise AssertionError(self.log.read())
            return json.loads(line)
        finally:selector.close()
    def call(self,command,body,deadline_ms=900,**replace):
        self.sequence+=1
        request={'schema':'rx.ros-jtc-request.v1','bridge_instance':self.instance,'sequence':str(self.sequence),'expires_at_ns':str(time.clock_gettime_ns(time.CLOCK_BOOTTIME)+int(deadline_ms*1e6)),'command':command,'body':body};request.update(replace)
        self.p.stdin.write(json.dumps(request)+'\n');self.p.stdin.flush();return self.read()
    def close(self):
        self.p.stdin.close();self.p.wait(timeout=5);assert self.p.returncode==0;self.log.close()
    def wait_services(self):
        for _ in range(30):
            info=self.call('inspect',{})
            if info['state']=='OBSERVED' and all(info['value'][k] for k in ['send_service_ready','result_service_ready','cancel_service_ready']):return info
            time.sleep(.05)
        raise AssertionError(info)

passed=[]
try:
    with tempfile.TemporaryDirectory(prefix='rx-ros-bridge-') as temporary:
        root=Path(temporary);b=Bridge(root);assert b.hello['value']['joints']==JOINTS
        assert state['received']==0
        for _ in range(20):
            info=b.call('inspect',{})
            if info['state']=='OBSERVED' and info['value']['send_service_ready'] and info['value']['result_service_ready'] and info['value']['cancel_service_ready']:break
            time.sleep(.05)
        assert info['value']['selected_matches'] and not info['value']['physical_readiness_proven'];passed.append('catalog_and_read_only_controller_inspection')
        for mutation in ['joint_order','partial','time','tolerance','extra']:
            g=goal()
            if mutation=='joint_order':g['joints']=list(reversed(JOINTS))
            if mutation=='partial':g['points'][0]['positions']=[.1]
            if mutation=='time':g['points'][0]['time_ns']='0'
            if mutation=='tolerance':g['goal_tolerance'][0]['position']=0
            if mutation=='extra':g['unexpected']=True
            reply=b.call('send',{'operation':str(uuid.uuid4()),'invocation':str(uuid.uuid4()),'goal':g});assert reply['state']=='REJECTED'
        assert state['received']==0;passed.append('invalid_trajectory_and_implicit_tolerances_send_nothing')
        for field in ['active','extra_claim']:
            state[field]=False if field=='active' else True
            reply=b.call('send',{'operation':str(uuid.uuid4()),'invocation':str(uuid.uuid4()),'goal':goal()});assert reply['state']=='REJECTED'
            state[field]=True if field=='active' else False
        assert state['received']==0;passed.append('inactive_or_different_joint_claims_rejected')
        op=str(uuid.uuid4());inv=str(uuid.uuid4());body={'operation':op,'invocation':inv,'goal':goal()}
        before=state['received'];sent=b.call('send',body);assert sent['state']=='SEND_RECORDED' and sent['value']['accepted']
        repeat=b.call('send',body);assert repeat['value']==sent['value'] and state['received']==before+1
        changed=dict(body);changed['goal']=goal(.2);assert b.call('send',changed)['state']=='REJECTED'
        assert b.call('send',{'operation':str(uuid.uuid4()),'invocation':str(uuid.uuid4()),'goal':goal()})['state']=='REJECTED'
        early=b.call('result',{'invocation':inv});assert early['state'] in ['RPC_UNKNOWN','RESULT_CAPTURED']
        time.sleep(.35);result=b.call('result',{'invocation':inv});assert result['state']=='RESULT_CAPTURED' and result['value']['ros_goal_status']==4 and result['value']['controller_error_code']==0
        time.sleep(.02);cached=b.call('result',{'invocation':inv});assert cached['value']==result['value'] and int(cached['ticks_ns'])>int(cached['value']['captured_at_ns'])
        assert inv in state['goals'];assert state['received']==before+1;passed.append('supplied_uuid_single_send_result_query_and_pending_admission')
        unknown=b.call('result',{'invocation':str(uuid.uuid4())});assert unknown['state']=='RESULT_UNKNOWN' and unknown['value']['ros_goal_status']==0;passed.append('unknown_goal_with_default_success_code_is_not_success')
        for value,status,code in [(.2,6,-4),(.5,4,-4),(.4,None,None)]:
            token=str(uuid.uuid4());reply=b.call('send',{'operation':str(uuid.uuid4()),'invocation':token,'goal':goal(value)})
            if value==.4:assert not reply['value']['accepted'];continue
            assert reply['value']['accepted'];time.sleep(.35);out=b.call('result',{'invocation':token});assert out['value']['ros_goal_status']==status and out['value']['controller_error_code']==code
        passed.append('native_rejection_abort_and_contradictory_result_preserved')
        token=str(uuid.uuid4());reply=b.call('send',{'operation':str(uuid.uuid4()),'invocation':token,'goal':goal(.3)});assert reply['value']['accepted']
        before_cancel=state['cancelled'];assert b.call('cancel',{'invocation':'00000000-0000-0000-0000-000000000000'})['state']=='REJECTED'
        assert b.call('cancel',{'invocation':str(uuid.uuid4())})['state']=='REJECTED'
        cancel_reply=b.call('cancel',{'invocation':token});assert cancel_reply['state']=='CANCEL_RESPONSE' and cancel_reply['value']['goals_canceling']==[token] and not cancel_reply['value']['terminal_stop_proven']
        assert b.call('cancel',{'invocation':token})['state']=='CANCEL_RECORDED';assert state['cancelled']==before_cancel+1
        time.sleep(.1);done=b.call('result',{'invocation':token});assert done['value']['ros_goal_status']==5;passed.append('exact_uuid_cancel_without_cancel_all_and_distinct_terminal_result')
        before=state['received'];state['list_delay']=.08
        rejected=b.call('send',{'operation':str(uuid.uuid4()),'invocation':str(uuid.uuid4()),'goal':goal()},deadline_ms=20)
        assert rejected['state'] in ['REJECTED','RPC_UNKNOWN'];state['list_delay']=0;time.sleep(.1);assert state['received']==before;passed.append('deadline_during_controller_preflight_has_no_native_send')
        assert b.call('inspect',{},bridge_instance=str(uuid.uuid4()))['state']=='REJECTED';assert b.call('inspect',{},sequence='1')['state']=='REJECTED'
        b.p.stdin.write('{"schema":1,"schema":2}\n');b.p.stdin.flush();assert b.read()['state']=='REJECTED';passed.append('stale_context_sequence_and_duplicate_json_rejected')
        old_instance=b.instance;b.close();restarted=Bridge(root);assert restarted.instance!=old_instance;assert restarted.call('cancel',{'invocation':token})['state']=='REJECTED';restarted.close();passed.append('restart_changes_bridge_identity_and_does_not_adopt_cancel_authority')
        delayed=Bridge(root);delayed.wait_services();before=state['received'];token=str(uuid.uuid4());body={'operation':str(uuid.uuid4()),'invocation':token,'goal':goal(.6)}
        uncertain=delayed.call('send',body);assert uncertain['state']=='SEND_UNKNOWN',uncertain
        assert delayed.call('send',body)['value']['state']=='SEND_UNKNOWN'
        time.sleep(.7);assert state['received']==before+1
        fact=delayed.call('result',{'invocation':token});assert fact['value']['ros_goal_status']==4
        assert delayed.call('send',{'operation':str(uuid.uuid4()),'invocation':str(uuid.uuid4()),'goal':goal()})['state']=='REJECTED';delayed.close();passed.append('lost_send_reply_is_not_retried_and_late_result_does_not_clear_admission_fault')
        delayed=Bridge(root);delayed.wait_services();token=str(uuid.uuid4());assert delayed.call('send',{'operation':str(uuid.uuid4()),'invocation':token,'goal':goal(.3)})['value']['accepted']
        state['cancel_delay']=.3;before=state['cancelled'];assert delayed.call('cancel',{'invocation':token})['state']=='CANCEL_UNKNOWN'
        assert delayed.call('cancel',{'invocation':token})['state']=='CANCEL_RECORDED';time.sleep(.4);assert state['cancelled']==before+1
        assert delayed.call('result',{'invocation':token})['value']['ros_goal_status']==5;state['cancel_delay']=0.;delayed.close();passed.append('lost_cancel_reply_is_retained_and_exact_cancel_is_not_resent')
        follower=Bridge(root,support_id='OM-07');assert len(follower.hello['value']['joints'])==7 and follower.hello['value']['joints'][-1]=='rh_r1_joint';follower.close();passed.append('follower_controller_with_integrated_gripper_has_distinct_joint_set')
        for support,controller in [('OM-04','arm_controller'),('AS-01','joint_group_impedance_controller'),('OM-06','gripper_controller')]:
            cfg={'schema':'rx.ros-jtc-bridge.v1','catalog_sha256':CATALOG,'support_id':support,'controller':controller,'namespace':'/rx_test','controller_manager':'/rx_test/controller_manager','domain_id':DOMAIN,'timeout_ms':150,'capacity':32};path=root/'invalid.json';path.write_text(json.dumps(cfg));out=subprocess.run([a.binary,str(path)],capture_output=True,text=True,timeout=5);assert out.returncode!=0
        passed.append('leader_impedance_and_other_controller_types_not_mislabeled_as_jtc')
        cfg.update(support_id='OM-06',controller='arm_controller',catalog_sha256='0'*64);path.write_text(json.dumps(cfg));out=subprocess.run([a.binary,str(path)],capture_output=True,text=True,timeout=5);assert out.returncode!=0;passed.append('catalog_source_pin_mismatch_rejected')
        catalog=json.loads((Path(__file__).resolve().parents[2]/'catalogs/robotis-support.v1.json').read_text());declarations=[];before=state['received']
        for profile in catalog['profiles']:
            for controller in profile['controllers']:
                if profile['role'] in ['MANIPULATOR','FOLLOWER','MOBILE_BASE'] and controller['plugin']=='joint_trajectory_controller/JointTrajectoryController' and controller['command_interfaces']==['position'] and controller['joint_order']:
                    candidate=Bridge(root,support_id=profile['support_id'],controller=controller['name'])
                    assert candidate.hello['value']['joints']==controller['joint_order'] and candidate.hello['value']['catalog_sha256']==CATALOG
                    candidate.close();declarations.append({'support_id':profile['support_id'],'controller':controller['name'],'joint_count':len(controller['joint_order'])})
        assert len(declarations)==51 and state['received']==before;passed.append('all_51_catalog_jtc_declarations_select_without_sending_goals')
    a.evidence.parent.mkdir(parents=True,exist_ok=True);a.evidence.write_text(json.dumps({'schema':'rx.ros-jtc-bridge-test.v1','status':'PASS','cases':passed,'catalog_declarations':declarations,'server':state,'physical_equipment_used':False,'host_native_journal_connected':False},indent=2)+'\n');print(json.dumps({'status':'PASS','cases':len(passed),'declarations':len(declarations)}))
finally:
    executor.shutdown(timeout_sec=5);node.destroy_node();rclpy.shutdown();thread.join(timeout=5)
