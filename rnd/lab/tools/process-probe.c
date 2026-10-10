#include <libproc.h>
#include <sys/resource.h>
#include <stdio.h>
#include <stdlib.h>
#include <inttypes.h>
#include <mach/mach_time.h>
int main(int argc,char **argv){
    mach_timebase_info_data_t timebase={0};if(mach_timebase_info(&timebase)!=KERN_SUCCESS||!timebase.denom)return 1;
    printf("[");int count=0;
    for(int i=1;i<argc;i++){
        int pid=atoi(argv[i]);struct rusage_info_v4 usage={0};
        if(pid<=0||proc_pid_rusage(pid,RUSAGE_INFO_V4,(rusage_info_t*)&usage))continue;
        if(count++)printf(",");
        uint64_t user_ns=(uint64_t)((long double)usage.ri_user_time*timebase.numer/timebase.denom);
        uint64_t system_ns=(uint64_t)((long double)usage.ri_system_time*timebase.numer/timebase.denom);
        printf("{\"pid\":%d,\"started\":\"%" PRIu64 "\",\"userNs\":%" PRIu64 ",\"systemNs\":%" PRIu64 ",\"timebaseNumer\":%u,\"timebaseDenom\":%u,\"rss\":%" PRIu64 ",\"footprint\":%" PRIu64 ",\"kernelPeakFootprint\":%" PRIu64 ",\"diskReadBytes\":%" PRIu64 ",\"diskWriteBytes\":%" PRIu64 "}",pid,usage.ri_proc_start_abstime,user_ns,system_ns,timebase.numer,timebase.denom,usage.ri_resident_size,usage.ri_phys_footprint,usage.ri_lifetime_max_phys_footprint,usage.ri_diskio_bytesread,usage.ri_diskio_byteswritten);
    }printf("]\n");return 0;
}
